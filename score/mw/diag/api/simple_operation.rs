/********************************************************************************
 * Copyright (c) 2026 Contributors to the Eclipse Foundation
 *
 * See the NOTICE file(s) distributed with this work for additional
 * information regarding copyright ownership.
 *
 * This program and the accompanying materials are made available under the
 * terms of the Apache License Version 2.0 which is available at
 * https://www.apache.org/licenses/LICENSE-2.0
 *
 * SPDX-License-Identifier: Apache-2.0
 ********************************************************************************/

use common::DiagnosticReply;
use common::Result as DiagResult;
use operation::{
    ExecuteArguments, ExecutionControl, ExecutionEventKind, ExecutionHandle, ExecutionId,
    ExecutionStatus, ExecutionStatusDetails,
};

use std::sync::{Arc, Mutex};

/// Simplified operation trait. Such a simple operation can be configured as synchronous
/// or asynchronous operation but is never allowed to get executed concurrently.
pub trait SimpleOperation {
    /// Start the operation with the given input arguments.
    /// Returns a handle whose future resolves to the operation result.
    fn start(&mut self, input: ExecuteArguments) -> DiagResult<ExecutionHandle>;

    /// Stop the operation, optionally with the given input arguments
    /// Returns an optional diagnostic reply.
    fn stop(&mut self, input: Option<ExecuteArguments>) -> DiagResult<Option<DiagnosticReply>>;

    /// Optionally provide the current completion percentage
    fn completion_percentage(&self) -> Option<u8> {
        None
    }
}

/// Adapter that wraps a `SimpleOperation` but implements the full `Operation` trait,
/// bridging the simplified interface to the runtime's execution control model.
pub struct SimpleOperationAdapter {
    op_instance: Arc<Mutex<Box<dyn SimpleOperation + Send>>>,
    active_exec_id: Mutex<Option<ExecutionId>>,
}

impl SimpleOperationAdapter {
    #[must_use]
    pub fn from(instance: impl SimpleOperation + Send + 'static) -> Self {
        Self {
            op_instance: Arc::new(Mutex::new(Box::new(instance))),
            active_exec_id: Mutex::new(None),
        }
    }
}

impl operation::Operation for SimpleOperationAdapter {
    fn execute(
        &mut self,
        input: ExecuteArguments,
        mut exec_control: ExecutionControl,
    ) -> DiagResult<ExecutionHandle> {
        let mut active_id = self
            .active_exec_id
            .lock()
            .map_err(|_| common::Error::mutex_error())?;
        if active_id.is_some() {
            return Err(common::Error::from_error(
                common::sovd::GenericError::from_code(
                    common::sovd::ErrorCode::PreconditionNotFulfilled,
                    "operation is already executing".to_string(),
                ),
            ));
        }
        *active_id = Some(exec_control.get_exec_id().clone());

        let op_instance = Arc::clone(&self.op_instance);

        let exec_handle = op_instance
            .lock()
            .map_err(|_| common::Error::mutex_error())?
            .start(input)?;

        let exec_control_future = async move {
            let mut last_exec_event_kind = ExecutionEventKind::Resume;
            let mut exec_errors: Vec<common::Error> = Vec::new();
            let mut exec_status = ExecutionStatus::Running;

            loop {
                let exec_event = exec_control.next_exec_event().await;
                match exec_event.kind {
                    ExecutionEventKind::ControlGone => break,

                    ExecutionEventKind::ReportStatus => {
                        let mut details =
                            ExecutionStatusDetails::from(last_exec_event_kind.clone());
                        if let Some(percentage) = op_instance
                            .lock()
                            .ok()
                            .and_then(|op| op.completion_percentage())
                        {
                            details = details.with_completion_percentage(percentage);
                        }
                        if !exec_errors.is_empty() {
                            details = details.with_exec_errors(exec_errors.clone());
                        }
                        exec_event.status_reporter.put(exec_status, details);
                    }

                    _ => {
                        last_exec_event_kind = exec_event.kind;
                        match last_exec_event_kind {
                            ExecutionEventKind::Stop => {
                                match op_instance
                                    .lock()
                                    .map_err(|_| common::Error::mutex_error())
                                    .and_then(|mut op| op.stop(exec_event.args))
                                {
                                    Ok(Some(result)) => {
                                        exec_status = ExecutionStatus::Stopped;
                                        exec_event.status_reporter.put(
                                            exec_status,
                                            ExecutionStatusDetails::none().with_reply_data(result),
                                        );
                                    }
                                    Ok(None) => {
                                        exec_status = ExecutionStatus::Stopped;
                                    }
                                    Err(err) => {
                                        exec_errors.push(err);
                                        exec_event.status_reporter.put(
                                            exec_status,
                                            ExecutionStatusDetails::none()
                                                .with_exec_errors(exec_errors.clone()),
                                        );
                                    }
                                }
                            }

                            _ => exec_status = ExecutionStatus::UnsupportedCapability,
                        }
                    }
                }
            }

            Ok(DiagnosticReply::default())
        };

        Ok(ExecutionHandle {
            future: Box::pin(async move {
                tokio::select! {
                    op_result = exec_handle.future => op_result,
                    ctrl_result = exec_control_future => ctrl_result,
                }
            }),
            reply: exec_handle.reply,
        })
    }
}
