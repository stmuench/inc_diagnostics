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

use diag_api::sovd::data_resource::*;
use diag_api::sovd::operation::*;
use diag_api::uds::{
    ReadDataByIdentifier, RoutineControl, RoutineControlAdapter, RoutineHandler,
    SerializedReadDataByIdentifier, SerializedRoutineControl, SerializedWriteDataByIdentifier,
    StartRoutine, UdsDeserialize, UdsSerialize, WriteHandler,
};
use diag_api::Result as DiagResult;
use diag_api::*;
use serde::{Deserialize, Serialize};

use diag_runtime::*;

use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::{mpsc, Notify};

/*************************************/
/* user implementation of a UDS RDBI */
/*************************************/

struct MyReadDataByIdentifier {}

impl ReadDataByIdentifier for MyReadDataByIdentifier {
    fn read(&self) -> DiagResult<Vec<u8>> {
        Ok(vec![0xDE, 0xAD, 0xBE, 0xEF])
    }
}

/**************************************************************/
/* UDS serialization types for SerializedReadDataByIdentifier */
/* and SerializedWriteDataByIdentifier                        */
/**************************************************************/

/// Two-byte big-endian vehicle speed value
struct VehicleSpeed {
    value: u16,
}

impl UdsSerialize for VehicleSpeed {
    fn serialize(&self) -> DiagResult<Vec<u8>> {
        Ok(self.value.to_be_bytes().to_vec())
    }
}

impl UdsDeserialize for VehicleSpeed {
    fn deserialize(data: &[u8]) -> DiagResult<Self> {
        if data.len() < 2 {
            return Err(diag_api::Error::from_nrc(
                diag_api::uds::NegativeResponseCode::IncorrectMessageLengthOrInvalidFormat,
            ));
        }
        Ok(Self { value: u16::from_be_bytes([data[0], data[1]]) })
    }
}

struct SpeedWriteHandler;

impl WriteHandler<VehicleSpeed> for SpeedWriteHandler {
    fn handle_write(&mut self, data: VehicleSpeed) -> DiagResult<()> {
        println!("SpeedWriteHandler: received speed = {} km/h", data.value);
        Ok(())
    }
}

/********************************************************/
/* SOVD JSON DataResource using serde serialization     */
/********************************************************/

/// SOVD JSON data resource serialized via `serde_json` on read/write.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct EngineStatus {
    running: bool,
    temperature_celsius: i32,
}

struct SovdEngineStatusResource {
    status: EngineStatus,
}

impl DataResource for SovdEngineStatusResource {
    fn read(&self, input: ReadValueArgs) -> ReadValueHandle {
        if !matches!(input.reply_encoding, ReplyMessageEncoding::JSON(_)) {
            return ReadValueHandle::from_error(diag_api::Error::from_error(
                diag_api::sovd::GenericError::from_code(
                    diag_api::sovd::ErrorCode::PreconditionNotFulfilled,
                    "This SOVD data resource only supports JSON encoding for its reply data!"
                        .to_string(),
                ),
            ));
        }
        match serde_json::to_value(&self.status) {
            Ok(json) => ReadValueHandle::ready(ReadValueReply {
                data: ReplyMessagePayload::from_json(json, None),
                errors: None,
            }),
            Err(e) => ReadValueHandle::from_error(diag_api::Error::from_error(
                diag_api::sovd::GenericError::from_code(
                    diag_api::sovd::ErrorCode::SovdServerFailure,
                    e.to_string(),
                ),
            )),
        }
    }

    fn write(&mut self, input: WriteValueArgs) -> WriteValueHandle {
        let Some(RequestMessagePayload::JSON(json)) = input.user_data else {
            return WriteValueHandle::from_error(DataError::from_error(
                diag_api::sovd::GenericError::from_code(
                    diag_api::sovd::ErrorCode::IncompleteRequest,
                    "expected JSON payload".to_string(),
                ),
            ));
        };
        match serde_json::from_value::<EngineStatus>(json) {
            Ok(new_status) => { self.status = new_status; WriteValueHandle::ready() }
            Err(e) => WriteValueHandle::from_error(DataError::from_error(
                diag_api::sovd::GenericError::from_code(
                    diag_api::sovd::ErrorCode::InvalidResponseContent,
                    e.to_string(),
                ),
            )),
        }
    }
}

/********************************************/
/* user implementation of a UDS routine     */
/********************************************/

struct MyUdsRoutine {
    completion: Arc<Notify>,
}

impl RoutineControl for MyUdsRoutine {
    fn start(&mut self, _input: Option<&[u8]>) -> DiagResult<StartRoutine> {
        let completion = self.completion.clone();
        StartRoutine::from_future(
            async move {
                completion.notified().await;
                Ok(Some(vec![0xCA, 0xFE]))
            },
            Some(vec![0xBE, 0xEF]),
        )
    }

    fn stop(&mut self, _input: Option<&[u8]>) -> DiagResult<Option<Vec<u8>>> {
        self.completion.notify_one();
        Ok(Some(vec![0xDE, 0xAD]))
    }
}

/******************************************/
/* user implementation of a data resource */
/******************************************/

struct MyDataResource {
    value: String,
}

impl DataResource for MyDataResource {
    fn read(&self, input: ReadValueArgs) -> ReadValueHandle {
        assert_eq!(input.reply_encoding, ReplyMessageEncoding::UTF8);
        ReadValueHandle::ready(ReadValueReply {
            data: ReplyMessagePayload::UTF8(self.value.clone()),
            errors: None,
        })
    }
}

/*************************************/
/* user implementation of operations */
/*************************************/

struct MySyncOperation {}

impl Operation for MySyncOperation {
    fn execute(
        &mut self,
        input: ExecuteArguments,
        _control: Box<dyn ExecutionControl>,
    ) -> DiagResult<ExecutionHandle> {
        assert_eq!(input.reply_encoding, ReplyMessageEncoding::UTF8);
        Ok(ExecutionHandle::from_closure(move || {
            println!("Sync operation execution got initiated ...");
            ExecutionResult::Ok(DiagnosticReply {
                message_payload: Some(ReplyMessagePayload::UTF8(
                    "This was a synchronous SOVD operation!".to_string(),
                )),
                additional_attrs: None,
            })
        }))
    }
}

struct MyAsyncOperation {}

impl MyAsyncOperation {
    async fn user_code(
        _input: ExecuteArguments,
        mut notification: mpsc::Receiver<()>,
    ) -> ExecutionResult {
        println!("Async operation execution got initiated ...");

        /* some complex long-running business logic here */

        notification.recv().await;

        println!("Async operation's execution finished successfully!");

        ExecutionResult::Ok(DiagnosticReply {
            message_payload: Some(ReplyMessagePayload::UTF8(
                "This was an asynchronous SOVD operation!".to_string(),
            )),
            additional_attrs: None,
        })
    }

    async fn exec_control(mut control: Box<dyn ExecutionControl>, notifier: mpsc::Sender<()>) {
        let mut exec_status = ExecutionStatus::Scheduled;
        loop {
            let Some(exec_event) = control.next().await else {
                break;
            };
            match exec_event.kind {
                ExecutionEventKind::HandleCustomCapability(custom_capability) => {
                    println!("Async operation's execution received custom capability event!");
                    assert_eq!("An OEM specific capability", custom_capability);
                    exec_status = ExecutionStatus::UnsupportedCapability(custom_capability);
                }

                ExecutionEventKind::ReportStatus => {
                    println!("Async operation's execution received report status event!");
                    exec_event
                        .status_reporter
                        .put(exec_status.clone(), ExecutionStatusDetails::default());
                    match exec_status {
                        ExecutionStatus::Scheduled => {
                            exec_status = ExecutionStatus::Running;
                        }
                        _other => {
                            exec_status = ExecutionStatus::Running;
                            notifier
                                .send(())
                                .await
                                .expect("Signalling user code failed!") // wakeup above business logic
                        }
                    }
                }

                ExecutionEventKind::ControlGone => {
                    println!("Async operation's execution control channel is gone, terminating execution!");
                    break;
                }

                ExecutionEventKind::Stop => {
                    println!("Async operation execution received stop event!");
                    exec_event
                        .status_reporter
                        .put(ExecutionStatus::Stopped, ExecutionStatusDetails::default());
                    break;
                }

                _other => panic!("Unsupported ExecutionEventKind!"),
            }
        }
    }
}

impl Operation for MyAsyncOperation {
    fn execute(
        &mut self,
        input: ExecuteArguments,
        control: Box<dyn ExecutionControl>,
    ) -> DiagResult<ExecutionHandle> {
        let (notifier, notification) = mpsc::channel(10);
        Ok(ExecutionHandle::from_future(async move {
            tokio::select! {
               result_data = Self::user_code(input, notification) => result_data,
               _ = Self::exec_control(control, notifier) => ExecutionResult::Err(Error::from_error(diag_api::sovd::GenericError::from_code(
                   diag_api::sovd::ErrorCode::ErrorResponse,
                   "execution got stopped".to_string(),
               ))),
            }
        }))
    }
}

/**************************************************************/
/* SerializedRoutineControl handler with bidirectional serde  */
/**************************************************************/

/// Echoes params back; exercises the full binary deserialize→handler→serialize round-trip.
struct EchoSpeedRoutineHandler;

impl RoutineHandler<VehicleSpeed> for EchoSpeedRoutineHandler {
    fn start(&mut self, params: Option<VehicleSpeed>) -> DiagResult<Option<VehicleSpeed>> { Ok(params) }
    fn stop(&mut self,  params: Option<VehicleSpeed>) -> DiagResult<Option<VehicleSpeed>> { Ok(params) }
}

/**********************/
/* execute operations */
/**********************/

#[cfg(test)]
mod tests {
    use super::*;

    const ENTITY_ID: &str = "test_entity";
    const SYNC_OP_ID: &str = "my_sync_op";
    const ASYNC_OP_ID: &str = "my_async_op";
    const DATA_RESOURCE_ID: &str = "my_data_resource";
    const UDS_DATA_RESOURCE_ID: &str = "uds_data_resource";
    const UDS_ROUTINE_OP_ID: &str = "my_uds_routine_op";
    const SERIALIZED_RDBI_ID: &str = "serialized_speed_rdbi";
    const SERIALIZED_WDBI_ID: &str = "serialized_speed_wdbi";
    const SOVD_ENGINE_STATUS_ID: &str = "sovd_engine_status";
    const SERIALIZED_ROUTINE_OP_ID: &str = "serialized_speed_routine";

    fn setup_runtime() -> Runtime {
        let runtime = Runtime::new();

        let entity = runtime.get_or_create_entity(ENTITY_ID.to_string());

        entity.register_data_resource(
            MyDataResource {
                value: "This is an SOVD data resource value.".to_string(),
            },
            DATA_RESOURCE_ID.to_string(),
            DataResourceMetadata {
                id: DATA_RESOURCE_ID.to_string(),
                name: "My Data Resource".to_string(),
                translation_id: None,
                read_only: false,
                category: DataCategory::CurrentData,
                groups: None,
            },
        );
        entity.register_data_resource(
            diag_api::uds::DataResourceAdapter::new().with_rdbi(MyReadDataByIdentifier {}),
            UDS_DATA_RESOURCE_ID.to_string(),
            DataResourceMetadata {
                id: UDS_DATA_RESOURCE_ID.to_string(),
                name: "My UDS Data Resource".to_string(),
                translation_id: None,
                read_only: true,
                category: DataCategory::StoredData,
                groups: None,
            },
        );

        entity.register_operation(
            MySyncOperation {},
            SYNC_OP_ID.to_string(),
            OperationMetadata {
                proximity_proof_required: false,
                synchronous_execution: true,
                exclusive_execution: false,
                supported_modes: None,
            },
        );
        entity.register_operation(
            MyAsyncOperation {},
            ASYNC_OP_ID.to_string(),
            OperationMetadata {
                proximity_proof_required: false,
                synchronous_execution: false,
                exclusive_execution: false,
                supported_modes: None,
            },
        );

        // SerializedReadDataByIdentifier: VehicleSpeed value serialized automatically on read.
        entity.register_data_resource(
            diag_api::uds::DataResourceAdapter::new()
                .with_rdbi(SerializedReadDataByIdentifier::new(VehicleSpeed { value: 120 })),
            SERIALIZED_RDBI_ID.to_string(),
            DataResourceMetadata {
                id: SERIALIZED_RDBI_ID.to_string(),
                name: "Vehicle Speed (Serialized RDBI)".to_string(),
                translation_id: None,
                read_only: true,
                category: DataCategory::CurrentData,
                groups: None,
            },
        );

        // SerializedWriteDataByIdentifier: raw bytes decoded into VehicleSpeed before
        // forwarding to SpeedWriteHandler.
        entity.register_data_resource(
            diag_api::uds::DataResourceAdapter::new()
                .with_wdbi(SerializedWriteDataByIdentifier::<VehicleSpeed, _>::new(
                    SpeedWriteHandler,
                )),
            SERIALIZED_WDBI_ID.to_string(),
            DataResourceMetadata {
                id: SERIALIZED_WDBI_ID.to_string(),
                name: "Vehicle Speed Write (Serialized WDBI)".to_string(),
                translation_id: None,
                read_only: false,
                category: DataCategory::CurrentData,
                groups: None,
            },
        );

        // SerializedRoutineControl: deserializes binary input → EchoSpeedRoutineHandler
        // → serializes typed reply back to binary. Full bidirectional UDS binary serialization.
        entity.register_operation(
            SimpleOperationAdapter::new(RoutineControlAdapter::new(
                SerializedRoutineControl::<VehicleSpeed, _>::new(EchoSpeedRoutineHandler),
            )),
            SERIALIZED_ROUTINE_OP_ID.to_string(),
            OperationMetadata {
                proximity_proof_required: false,
                synchronous_execution: false,
                exclusive_execution: false,
                supported_modes: None,
            },
        );

        // SOVD JSON DataResource: read serializes via serde_json::to_value(),
        // write deserializes via serde_json::from_value().
        entity.register_data_resource(
            SovdEngineStatusResource {
                status: EngineStatus {
                    running: true,
                    temperature_celsius: 90,
                },
            },
            SOVD_ENGINE_STATUS_ID.to_string(),
            DataResourceMetadata {
                id: SOVD_ENGINE_STATUS_ID.to_string(),
                name: "Engine Status (SOVD JSON)".to_string(),
                translation_id: None,
                read_only: false,
                category: DataCategory::CurrentData,
                groups: None,
            },
        );

        runtime
    }

    //
    // list registered data resources and verify that the expected metadata as well as content is returned
    //
    #[tokio::test]
    async fn test_data_resource_read() {
        let runtime = setup_runtime();
        let runtime_join_handle = tokio::spawn(runtime.run());

        match runtime
            .send(SOVDMessage::ListDataResources(ENTITY_ID.to_string()))
            .await
        {
            SOVDReply::ListDataResources(Ok(resources)) => {
                assert_eq!(resources.len(), 5);
                assert_eq!(resources[0].id, DATA_RESOURCE_ID);
                assert_eq!(resources[0].name, "My Data Resource");
                assert_eq!(resources[0].category, DataCategory::CurrentData);
                assert_eq!(resources[1].id, UDS_DATA_RESOURCE_ID);
                assert_eq!(resources[1].name, "My UDS Data Resource");
                assert_eq!(resources[1].category, DataCategory::StoredData);
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        // read the data resource's value and verify it matches the predefined value
        match runtime
            .send(SOVDMessage::ReadDataResource(
                ENTITY_ID.to_string(),
                DATA_RESOURCE_ID.to_string(),
                ReadValueArgs::new(ReplyMessageEncoding::UTF8),
            ))
            .await
        {
            SOVDReply::ReadDataResource(Ok(read_value)) => {
                assert_eq!(
                    read_value.data,
                    ReplyMessagePayload::UTF8("This is an SOVD data resource value.".to_string())
                );
                assert!(read_value.errors.is_none());
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        runtime.shutdown().await;
        let _ = runtime_join_handle.await;
    }

    //
    // verify that a UDS RDBI data resource can be read successfully and that writing to it gets rejected
    //
    #[tokio::test]
    async fn test_data_resource_uds_rdbi() {
        let runtime = setup_runtime();
        let runtime_join_handle = tokio::spawn(runtime.run());

        // test successful UDS read
        match runtime
            .send(SOVDMessage::ReadDataResource(
                ENTITY_ID.to_string(),
                UDS_DATA_RESOURCE_ID.to_string(),
                ReadValueArgs::new(ReplyMessageEncoding::Binary),
            ))
            .await
        {
            SOVDReply::ReadDataResource(Ok(read_value)) => {
                assert_eq!(
                    read_value.data,
                    ReplyMessagePayload::Binary(vec![0xDE, 0xAD, 0xBE, 0xEF])
                );
                assert!(read_value.errors.is_none());
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        // verify error upon write (RDBI does not support write)
        let mut rdbi_resource =
            diag_api::uds::DataResourceAdapter::new().with_rdbi(MyReadDataByIdentifier {});
        let write_handle = rdbi_resource.write(WriteValueArgs {
            user_data_signature: None,
            user_data: None,
            additional_attrs: None,
        });
        let err = match write_handle {
            WriteValueHandle::Ready(result) => result.unwrap_err(),
            WriteValueHandle::Pending(_) => panic!("expected Ready, got Pending"),
        };
        assert_eq!(
            err.error.as_ref().unwrap().sovd_error,
            "precondition-not-fulfilled"
        );

        runtime.shutdown().await;
        let _ = runtime_join_handle.await;
    }

    //
    // trigger execution of a simple sync SOVD operation via the runtime
    //
    #[tokio::test]
    async fn test_execution_of_synchronous_operation() {
        println!();

        let runtime = setup_runtime();
        let runtime_join_handle = tokio::spawn(runtime.run());

        let exec_id = match runtime
            .send(SOVDMessage::ExecuteOperation((
                ENTITY_ID.to_string(),
                SYNC_OP_ID.to_string(),
                None,
            )))
            .await
        {
            SOVDReply::ExecuteOperation(Ok(id)) => id,
            other => panic!("Unexpected reply: {:?}", other),
        };
        println!("Sync operation executed, execution id: {}", exec_id);

        // check the execution result
        match runtime
            .send(SOVDMessage::GetOperationExecutionResult((
                ENTITY_ID.to_string(),
                SYNC_OP_ID.to_string(),
                exec_id.clone(),
            )))
            .await
        {
            SOVDReply::GetOperationExecutionResult(Ok(exec_result)) => {
                assert_eq!(
                    exec_result,
                    ExecutionResult::Ok(DiagnosticReply {
                        message_payload: Some(ReplyMessagePayload::UTF8(
                            "This was a synchronous SOVD operation!".to_string(),
                        )),
                        additional_attrs: None,
                    })
                );
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        runtime.shutdown().await;
        let _ = runtime_join_handle.await;

        println!("DONE");
    }

    //
    // trigger execution of an async SOVD operation via the runtime, then stop it
    //
    #[tokio::test]
    async fn test_execution_and_stop_of_asynchronous_operation() {
        println!();

        let runtime = setup_runtime();
        let runtime_join_handle = tokio::spawn(runtime.run());

        let exec_id = match runtime
            .send(SOVDMessage::ExecuteOperation((
                ENTITY_ID.to_string(),
                ASYNC_OP_ID.to_string(),
                None,
            )))
            .await
        {
            SOVDReply::ExecuteOperation(Ok(id)) => id,
            _other => panic!("Unexpected reply!"),
        };
        println!("Async operation executed, execution id: {}", exec_id);

        // stop the execution
        match runtime
            .send(SOVDMessage::StopOperationExecution((
                ENTITY_ID.to_string(),
                ASYNC_OP_ID.to_string(),
                exec_id.clone(),
            )))
            .await
        {
            SOVDReply::StopOperationExecution(Ok(())) => {
                println!("Async operation execution stopped successfully!");
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        // allow the spawned task to complete after stop
        tokio::task::yield_now().await;

        // check the execution result
        match runtime
            .send(SOVDMessage::GetOperationExecutionResult((
                ENTITY_ID.to_string(),
                ASYNC_OP_ID.to_string(),
                exec_id.clone(),
            )))
            .await
        {
            SOVDReply::GetOperationExecutionResult(Ok(exec_result)) => {
                let err = exec_result.expect_err("expected an error result after stop");
                assert_eq!(
                    err.code,
                    ErrorCode::SOVD(diag_api::sovd::GenericError::from_code(
                        diag_api::sovd::ErrorCode::ErrorResponse,
                        "execution got stopped".to_string(),
                    ))
                );
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        runtime.shutdown().await;
        let _ = runtime_join_handle.await;

        println!("DONE");
    }

    //
    // trigger execution of an async SOVD operation with a timeout that expires
    //
    #[tokio::test]
    async fn test_execution_timeout_of_asynchronous_operation() {
        println!();

        let runtime = setup_runtime();
        let runtime_join_handle = tokio::spawn(runtime.run());

        // execute with a 3-second timeout
        let exec_id = match runtime
            .send(SOVDMessage::ExecuteOperation((
                ENTITY_ID.to_string(),
                ASYNC_OP_ID.to_string(),
                Some(std::time::Duration::from_secs(3)),
            )))
            .await
        {
            SOVDReply::ExecuteOperation(Ok(id)) => id,
            other => panic!("Unexpected reply: {:?}", other),
        };
        println!(
            "Async operation executed with 3s timeout, execution id: {}",
            exec_id
        );

        // wait for the timeout to expire (+ a small margin)
        tokio::time::sleep(std::time::Duration::from_millis(3250)).await;

        // check the execution result — should be `Stopped` due to timeout
        match runtime
            .send(SOVDMessage::GetOperationExecutionResult((
                ENTITY_ID.to_string(),
                ASYNC_OP_ID.to_string(),
                exec_id.clone(),
            )))
            .await
        {
            SOVDReply::GetOperationExecutionResult(Ok(exec_result)) => {
                let err = exec_result.expect_err("expected an error result after timeout");
                assert_eq!(
                    err.code,
                    ErrorCode::SOVD(diag_api::sovd::GenericError::from_code(
                        diag_api::sovd::ErrorCode::ErrorResponse,
                        "execution got stopped due to timeout".to_string(),
                    ))
                );
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        runtime.shutdown().await;
        let _ = runtime_join_handle.await;

        println!("DONE");
    }

    //
    // trigger execution of an async SOVD operation via the runtime
    //
    #[tokio::test]
    async fn test_execution_and_query_of_asynchronous_operation() {
        println!();

        let runtime = setup_runtime();
        let runtime_join_handle = tokio::spawn(runtime.run());

        let exec_id = match runtime
            .send(SOVDMessage::ExecuteOperation((
                ENTITY_ID.to_string(),
                ASYNC_OP_ID.to_string(),
                None,
            )))
            .await
        {
            SOVDReply::ExecuteOperation(Ok(id)) => id,
            _other => panic!("Unexpected reply!"),
        };
        println!("Async operation executed, execution id: {}", exec_id);

        // request current status of the execution (should be `Scheduled`)
        match runtime
            .send(SOVDMessage::GetOperationExecutionStatus((
                ENTITY_ID.to_string(),
                ASYNC_OP_ID.to_string(),
                exec_id.clone(),
            )))
            .await
        {
            SOVDReply::GetOperationExecutionStatus(Ok(status)) => {
                assert_eq!(status, ExecutionStatus::Scheduled);
                println!("Execution status: {:?}", status);
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        // request processing of a custom capability
        match runtime
            .send(SOVDMessage::ExecuteOperationCapability((
                ENTITY_ID.to_string(),
                ASYNC_OP_ID.to_string(),
                exec_id.clone(),
                "An OEM specific capability".to_string(),
            )))
            .await
        {
            SOVDReply::ExecuteOperationCapability(Ok(())) => {
                println!("Custom capability processed!");
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        // request current status (should result in UnsupportedCapability, which also signals user_code)
        match runtime
            .send(SOVDMessage::GetOperationExecutionStatus((
                ENTITY_ID.to_string(),
                ASYNC_OP_ID.to_string(),
                exec_id.clone(),
            )))
            .await
        {
            SOVDReply::GetOperationExecutionStatus(Ok(status)) => {
                assert_eq!(
                    status,
                    ExecutionStatus::UnsupportedCapability(
                        "An OEM specific capability".to_string()
                    )
                );
                println!("Execution status: {:?}", status);
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        // request current status once more (should be `Running` now)
        match runtime
            .send(SOVDMessage::GetOperationExecutionStatus((
                ENTITY_ID.to_string(),
                ASYNC_OP_ID.to_string(),
                exec_id.clone(),
            )))
            .await
        {
            SOVDReply::GetOperationExecutionStatus(Ok(status)) => {
                assert_eq!(status, ExecutionStatus::Running);
                println!("Execution status: {:?}", status);
            }
            // execution may have already finished
            SOVDReply::GetOperationExecutionStatus(Err(_)) => {
                println!("Execution already finished");
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        // allow the spawned task to complete
        tokio::task::yield_now().await;

        // check the execution result
        match runtime
            .send(SOVDMessage::GetOperationExecutionResult((
                ENTITY_ID.to_string(),
                ASYNC_OP_ID.to_string(),
                exec_id.clone(),
            )))
            .await
        {
            SOVDReply::GetOperationExecutionResult(Ok(exec_result)) => {
                assert_eq!(
                    exec_result,
                    ExecutionResult::Ok(DiagnosticReply {
                        message_payload: Some(ReplyMessagePayload::UTF8(
                            "This was an asynchronous SOVD operation!".to_string(),
                        )),
                        additional_attrs: None,
                    })
                );
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        runtime.shutdown().await;
        let _ = runtime_join_handle.await;

        println!("DONE");
    }

    //
    // trigger execution of an operation which got implemented via the `uds::RoutineControl`
    // API, then stop it and check the execution result
    //
    #[tokio::test]
    async fn test_execution_and_query_of_uds_routine() {
        println!();

        let runtime = setup_runtime();
        let runtime_join_handle = tokio::spawn(runtime.run());

        // register a UDS RoutineControl-based operation
        let routine_completion = Arc::new(Notify::new());
        let entity = runtime.get_or_create_entity(ENTITY_ID.to_string());
        entity.register_operation(
            SimpleOperationAdapter::new(RoutineControlAdapter::new(MyUdsRoutine {
                completion: routine_completion.clone(),
            })),
            UDS_ROUTINE_OP_ID.to_string(),
            OperationMetadata {
                proximity_proof_required: false,
                synchronous_execution: false,
                exclusive_execution: false,
                supported_modes: None,
            },
        );

        // execute the routine
        let exec_id = match runtime
            .send(SOVDMessage::ExecuteOperation((
                ENTITY_ID.to_string(),
                UDS_ROUTINE_OP_ID.to_string(),
                None,
            )))
            .await
        {
            SOVDReply::ExecuteOperation(Ok(id)) => id,
            other => panic!("Unexpected reply: {:?}", other),
        };
        println!("UDS routine executed, execution id: {}", exec_id);

        // request current status of the execution (should be `Running`)
        match runtime
            .send(SOVDMessage::GetOperationExecutionStatus((
                ENTITY_ID.to_string(),
                UDS_ROUTINE_OP_ID.to_string(),
                exec_id.clone(),
            )))
            .await
        {
            SOVDReply::GetOperationExecutionStatus(Ok(status)) => {
                assert_eq!(status, ExecutionStatus::Running);
                println!("Execution status: {:?}", status);
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        // stop the execution
        match runtime
            .send(SOVDMessage::StopOperationExecution((
                ENTITY_ID.to_string(),
                UDS_ROUTINE_OP_ID.to_string(),
                exec_id.clone(),
            )))
            .await
        {
            SOVDReply::StopOperationExecution(Ok(())) => {
                println!("UDS routine execution stopped successfully!");
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        // allow the spawned task to complete after stop
        tokio::task::yield_now().await;

        // check the execution result
        match runtime
            .send(SOVDMessage::GetOperationExecutionResult((
                ENTITY_ID.to_string(),
                UDS_ROUTINE_OP_ID.to_string(),
                exec_id.clone(),
            )))
            .await
        {
            SOVDReply::GetOperationExecutionResult(Ok(exec_result)) => {
                assert_eq!(
                    exec_result,
                    ExecutionResult::Ok(DiagnosticReply {
                        message_payload: Some(ReplyMessagePayload::Binary(vec![0xCA, 0xFE])),
                        additional_attrs: None,
                    })
                );
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        runtime.shutdown().await;
        let _ = runtime_join_handle.await;

        println!("DONE");
    }

    // SerializedReadDataByIdentifier: VehicleSpeed{120} → [0x00,0x78] via binary read.
    // SerializedWriteDataByIdentifier: [0x00,0x64] → VehicleSpeed{100} → handler.
    #[tokio::test]
    async fn test_serialized_rdbi_read_and_wdbi_write() {
        let runtime = setup_runtime();
        let runtime_join_handle = tokio::spawn(runtime.run());

        // --- SerializedReadDataByIdentifier: binary read returns big-endian u16 bytes ---
        match runtime
            .send(SOVDMessage::ReadDataResource(
                ENTITY_ID.to_string(),
                SERIALIZED_RDBI_ID.to_string(),
                ReadValueArgs::new(ReplyMessageEncoding::Binary),
            ))
            .await
        {
            SOVDReply::ReadDataResource(Ok(read_value)) => {
                // VehicleSpeed { value: 120 } serializes to [0x00, 0x78]
                assert_eq!(read_value.data, ReplyMessagePayload::Binary(vec![0x00, 0x78]));
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        // --- SerializedWriteDataByIdentifier: raw bytes decoded to VehicleSpeed, handler called ---
        // [0x00, 0x64] decodes to VehicleSpeed { value: 100 } and reaches the handler
        let mut wdbi = diag_api::uds::DataResourceAdapter::new()
            .with_wdbi(SerializedWriteDataByIdentifier::<VehicleSpeed, _>::new(SpeedWriteHandler));
        match wdbi.write(WriteValueArgs {
            user_data_signature: None,
            user_data: Some(RequestMessagePayload::Binary(vec![0x00, 0x64])),
            additional_attrs: None,
        }) {
            WriteValueHandle::Ready(result) => result.expect("write must succeed"),
            WriteValueHandle::Pending(_) => panic!("expected Ready"),
        }

        runtime.shutdown().await;
        let _ = runtime_join_handle.await;
    }

    // SOVD JSON: read → serde_json::to_value(); write → serde_json::from_value().
    #[tokio::test]
    async fn test_sovd_json_engine_status_read_and_write() {
        let runtime = setup_runtime();
        let runtime_join_handle = tokio::spawn(runtime.run());

        match runtime
            .send(SOVDMessage::ReadDataResource(
                ENTITY_ID.to_string(),
                SOVD_ENGINE_STATUS_ID.to_string(),
                ReadValueArgs::new(ReplyMessageEncoding::JSON(JsonSchemaRequired::No)),
            ))
            .await
        {
            SOVDReply::ReadDataResource(Ok(read_value)) => {
                assert_eq!(
                    read_value.data,
                    ReplyMessagePayload::from_json(
                        serde_json::json!({ "running": true, "temperature_celsius": 90 }),
                        None
                    )
                );
            }
            other => panic!("Unexpected reply: {:?}", other),
        }

        let mut resource = SovdEngineStatusResource {
            status: EngineStatus { running: true, temperature_celsius: 90 },
        };

        // Valid JSON updates the resource state.
        match resource.write(WriteValueArgs {
            user_data_signature: None,
            user_data: Some(RequestMessagePayload::JSON(
                serde_json::json!({ "running": false, "temperature_celsius": 75 }),
            )),
            additional_attrs: None,
        }) {
            WriteValueHandle::Ready(result) => {
                result.expect("write must succeed");
                assert_eq!(resource.status, EngineStatus { running: false, temperature_celsius: 75 });
            }
            WriteValueHandle::Pending(_) => panic!("expected Ready"),
        }

        // Invalid JSON shape must be rejected.
        match resource.write(WriteValueArgs {
            user_data_signature: None,
            user_data: Some(RequestMessagePayload::JSON(serde_json::json!({ "unknown": 0 }))),
            additional_attrs: None,
        }) {
            WriteValueHandle::Ready(result) => result.expect_err("invalid JSON must be rejected"),
            WriteValueHandle::Pending(_) => panic!("expected Ready"),
        };

        // Non-JSON encoding must be rejected — this data resource is JSON-only.
        match SovdEngineStatusResource { status: EngineStatus { running: true, temperature_celsius: 90 } }
            .read(ReadValueArgs::new(ReplyMessageEncoding::Binary))
        {
            ReadValueHandle::Ready(Err(err)) => match err.code {
                ErrorCode::SOVD(ref e) => assert_eq!(e.sovd_error, "precondition-not-fulfilled"),
                _ => panic!("expected SOVD error"),
            },
            _ => panic!("expected Ready(Err(..))"),
        }

        runtime.shutdown().await;
        let _ = runtime_join_handle.await;
    }

    // SerializedRoutineControl: binary → deserialize → handler → serialize → binary (bidirectional).
    #[test]
    fn test_serialized_routine_control_bidirectional() {
        let mut routine = SerializedRoutineControl::<VehicleSpeed, _>::new(EchoSpeedRoutineHandler);

        // [0x00, 0x96] = VehicleSpeed { value: 150 } — echoed back as serialized reply
        let start = routine.start(Some(&[0x00, 0x96])).expect("start must succeed");
        assert_eq!(start.reply, Some(vec![0x00, 0x96]));

        // [0x00, 0x64] = VehicleSpeed { value: 100 }
        assert_eq!(routine.stop(Some(&[0x00, 0x64])).expect("stop must succeed"), Some(vec![0x00, 0x64]));

        // Too-short input must be rejected.
        assert_eq!(
            routine.start(Some(&[0x01])).unwrap_err().code,
            diag_api::ErrorCode::UDS(
                diag_api::uds::NegativeResponseCode::IncorrectMessageLengthOrInvalidFormat
            )
        );
    }
}