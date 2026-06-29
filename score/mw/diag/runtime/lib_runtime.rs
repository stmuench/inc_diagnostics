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

use diag_api::sovd::data_resource::{
    DataResource, DataResourceMetadata, ReadValueArgs, ReadValueReply,
};
use diag_api::sovd::operation::{
    ExecuteArguments, ExecutionControl, ExecutionControlApi, ExecutionEvent, ExecutionEventKind,
    ExecutionResult, ExecutionStatus, Operation, OperationMetadata,
};
use diag_api::sovd::app_registration::{
    AppRegistrar, AppRegistryQuery, DeregisterAppArgs, RegisterAppArgs, RegisterAppReply,
};
use diag_api::Error as DiagError;
use diag_api::Result as DiagResult;
use diag_api::*;

use futures::future::BoxFuture;
use futures::FutureExt;

use indexmap::IndexMap;

use tokio::sync::{mpsc, oneshot};

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod opensovd_registrar;

pub use opensovd_registrar::OpenSovdRegistrar;

pub type EntityId = String;
pub type ExecutionId = String;
pub type OperationId = String;
pub type DataResourceId = String;
pub type ExecutionTimeout = Duration;

const DEFAULT_REGISTRATION_LEASE_MS: u64 = 30_000;

static EXECUTION_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn issue_new_execution_id() -> ExecutionId {
    EXECUTION_ID_COUNTER
        .fetch_add(1, Ordering::Relaxed)
        .to_string()
}

// this message type is for demonstration purposes only
#[derive(Debug)]
pub enum SOVDMessage {
    ListDataResources(EntityId),
    ReadDataResource(EntityId, DataResourceId, ReadValueArgs),
    ListOperations(EntityId),
    GetOperationMetadata((EntityId, OperationId)),
    ExecuteOperation((EntityId, OperationId, Option<ExecutionTimeout>)),
    ExecuteOperationCapability((EntityId, OperationId, ExecutionId, String)),
    GetOperationExecutionStatus((EntityId, OperationId, ExecutionId)),
    GetOperationExecutionResult((EntityId, OperationId, ExecutionId)),
    StopOperationExecution((EntityId, OperationId, ExecutionId)),
    RemoveOperationExecution((EntityId, OperationId, ExecutionId)),
}

// this message type is for demonstration purposes only
#[derive(Debug)]
pub enum SOVDReply {
    ListDataResources(DiagResult<Vec<DataResourceMetadata>>),
    ReadDataResource(DiagResult<ReadValueReply>),
    ListOperations(DiagResult<Vec<OperationMetadata>>),
    GetOperationMetadata(DiagResult<OperationMetadata>),
    ExecuteOperation(DiagResult<ExecutionId>),
    ExecuteOperationCapability(DiagResult<()>),
    GetOperationExecutionStatus(DiagResult<ExecutionStatus>),
    GetOperationExecutionResult(DiagResult<ExecutionResult>),
    StopOperationExecution(DiagResult<()>),
    RemoveOperationExecution(DiagResult<()>),
}

/***********************************/
/* ExecutionControl for operations */
/***********************************/

struct ExecutionControlImpl {
    exec_events: mpsc::Receiver<ExecutionEvent>,
}

impl ExecutionControlImpl {
    fn new(exec_events: mpsc::Receiver<ExecutionEvent>) -> Self {
        Self { exec_events }
    }

    async fn do_get_next_event(&mut self) -> ExecutionEvent {
        self.exec_events
            .recv()
            .map(|event| {
                event.unwrap_or(ExecutionEvent::from_kind(ExecutionEventKind::ControlGone))
            })
            .await
    }
}

impl ExecutionControlApi for ExecutionControlImpl {
    fn next_exec_event(&mut self) -> BoxFuture<'_, ExecutionEvent> {
        Box::pin(self.do_get_next_event())
    }
}

/**********************/
/* Active executions  */
/**********************/

enum ActiveExecution {
    Completed(ExecutionResult),
    Running {
        exec_control: mpsc::Sender<ExecutionEvent>,
        join_handle: Option<tokio::task::JoinHandle<ExecutionResult>>,
    },
}

impl ActiveExecution {
    fn try_resolve(&mut self) {
        if let ActiveExecution::Running {
            join_handle: Some(handle),
            ..
        } = self
        {
            if handle.is_finished() {
                let old = std::mem::replace(
                    self,
                    ActiveExecution::Completed(Err(DiagError::from_error(
                        sovd::GenericError::from_code(
                            sovd::ErrorCode::SovdServerFailure,
                            "execution result unavailable".to_string(),
                        ),
                    ))),
                );
                if let ActiveExecution::Running {
                    join_handle: Some(handle),
                    ..
                } = old
                {
                    if let Some(Ok(result)) = handle.now_or_never() {
                        *self = ActiveExecution::Completed(result);
                    }
                }
            }
        }
    }

    fn get_exec_control(&self) -> DiagResult<mpsc::Sender<ExecutionEvent>> {
        match self {
            ActiveExecution::Running { exec_control, .. } => Ok(exec_control.clone()),
            ActiveExecution::Completed(_) => {
                Err(DiagError::from_error(sovd::GenericError::from_code(
                    sovd::ErrorCode::PreconditionNotFulfilled,
                    "execution is already completed".to_string(),
                )))
            }
        }
    }

    fn get_result(&self) -> DiagResult<ExecutionResult> {
        match self {
            ActiveExecution::Completed(result) => Ok(result.clone()),
            ActiveExecution::Running { .. } => {
                Err(DiagError::from_error(sovd::GenericError::from_code(
                    sovd::ErrorCode::PreconditionNotFulfilled,
                    "execution is still running".to_string(),
                )))
            }
        }
    }
}

/***********/
/* Runtime */
/***********/

pub struct Runtime {
    inner: Arc<Mutex<RuntimeImpl>>,
}

#[derive(Clone, Debug)]
struct RegisteredApp {
    endpoint: String,
    registration_id: Option<String>,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RuntimeImpl::new())),
        }
    }

    pub fn with_registrar_backend(registrar_backend: Arc<dyn AppRegistrar + Send + Sync>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RuntimeImpl::with_registrar_backend(registrar_backend))),
        }
    }

    pub fn get_or_create_entity(&self, id: EntityId) -> Arc<Entity> {
        self.inner
            .lock()
            .expect("mutex acquisition failed")
            .get_or_create_entity(id)
    }

    pub fn run(&self) -> impl Future<Output = ()> {
        self.inner.lock().expect("mutex acquisition failed").run()
    }

    // This method is for demonstration purposes only!
    pub fn send(&self, message: SOVDMessage) -> impl Future<Output = SOVDReply> {
        self.inner
            .lock()
            .expect("mutex acquisition failed")
            .send(message)
    }

    pub fn shutdown(&self) -> impl Future<Output = ()> {
        self.inner
            .lock()
            .expect("mutex acquisition failed")
            .shutdown()
    }

    pub fn unregister_entity(&self, id: &EntityId) -> DiagResult<()> {
        self.inner
            .lock()
            .map_err(|_| DiagError::mutex_error())?
            .unregister_entity(id)
    }
}

impl AppRegistrar for Runtime {
    fn register_app(&self, args: RegisterAppArgs) -> BoxFuture<'_, DiagResult<RegisterAppReply>> {
        async move {
            let registrar_backend = self
                .inner
                .lock()
                .map_err(|_| DiagError::mutex_error())?
                .registrar_backend
                .clone();

            let backend_reply = match registrar_backend {
                Some(registrar_backend) => Some(registrar_backend.register_app(args.clone()).await?),
                None => None,
            };

            self.inner
                .lock()
                .map_err(|_| DiagError::mutex_error())?
                .register_app_locally(args, backend_reply)
        }
        .boxed()
    }

    fn deregister_app(&self, args: DeregisterAppArgs) -> BoxFuture<'_, DiagResult<()>> {
        async move {
            let (registrar_backend, resolved_args) = {
                let runtime = self.inner.lock().map_err(|_| DiagError::mutex_error())?;
                let resolved_args = DeregisterAppArgs {
                    registration_id: args
                        .registration_id
                        .clone()
                        .or_else(|| runtime.registration_id_for(&args.app_id)),
                    ..args.clone()
                };
                (runtime.registrar_backend.clone(), resolved_args)
            };

            if let Some(registrar_backend) = registrar_backend {
                registrar_backend.deregister_app(resolved_args.clone()).await?;
            }

            self.inner
                .lock()
                .map_err(|_| DiagError::mutex_error())?
                .deregister_app_locally(resolved_args)
        }
        .boxed()
    }
}

impl AppRegistryQuery for Runtime {
    fn resolve_endpoint(&self, app_id: &str) -> BoxFuture<'_, DiagResult<ReplyMessagePayload>> {
        let app_id = app_id.to_string();
        async move {
            self.inner
                .lock()
                .map_err(|_| DiagError::mutex_error())?
                .resolve_endpoint(&app_id)
        }
        .boxed()
    }
}

struct RuntimeImpl {
    entities: Arc<Mutex<IndexMap<EntityId, Arc<Entity>>>>,
    registrations: IndexMap<String, RegisteredApp>,
    registrar_backend: Option<Arc<dyn AppRegistrar + Send + Sync>>,
    registration_counter: u64,
    sovd_sender: mpsc::Sender<(SOVDMessage, oneshot::Sender<SOVDReply>)>,
    sovd_messages: Option<mpsc::Receiver<(SOVDMessage, oneshot::Sender<SOVDReply>)>>,
    request_shutdown: Option<oneshot::Sender<()>>,
    shutdown_requested: Option<oneshot::Receiver<()>>,
}

impl RuntimeImpl {
    pub fn new() -> Self {
        Self::with_optional_backends(None)
    }

    pub fn with_registrar_backend(registrar_backend: Arc<dyn AppRegistrar + Send + Sync>) -> Self {
        Self::with_optional_backends(Some(registrar_backend))
    }

    fn with_optional_backends(
        registrar_backend: Option<Arc<dyn AppRegistrar + Send + Sync>>,
    ) -> Self {
        let (sovd_sender, sovd_receiver) =
            mpsc::channel::<(SOVDMessage, oneshot::Sender<SOVDReply>)>(10);
        let (shutdown_sender, shutdown_receiver) = oneshot::channel();
        RuntimeImpl {
            entities: Arc::new(Mutex::new(IndexMap::<EntityId, Arc<Entity>>::new())),
            registrations: IndexMap::new(),
            registrar_backend,
            registration_counter: 0,
            sovd_sender,
            sovd_messages: Some(sovd_receiver),
            request_shutdown: Some(shutdown_sender),
            shutdown_requested: Some(shutdown_receiver),
        }
    }

    pub fn run(&mut self) -> impl Future<Output = ()> {
        let shutdown_requested = self
            .shutdown_requested
            .take()
            .expect("Runtime got started already!");
        let mut sovd_messages = self
            .sovd_messages
            .take()
            .expect("Runtime got started already!");
        let entities = self.entities.clone();

        async move {
            let shutdown_request_received = async move {
                let _ = shutdown_requested.await;
            };

            let main_loop = async move {
                loop {
                    match sovd_messages.recv().await {
                        Some((message, reply_sender)) => {
                            let reply = Self::handle(&entities, message).await;
                            let _ = reply_sender.send(reply);
                        }
                        None => break,
                    }
                }
            };

            tokio::select! {
                _ = shutdown_request_received => (),
                _ = main_loop => (),
            }
        }
    }

    async fn handle(
        entities: &Mutex<IndexMap<EntityId, Arc<Entity>>>,
        message: SOVDMessage,
    ) -> SOVDReply {
        match message {
            SOVDMessage::ListDataResources(entity_id) => {
                let reply = Self::select_entity(entities, &entity_id, |entity| {
                    Ok(entity.list_data_resources())
                });
                SOVDReply::ListDataResources(reply)
            }

            SOVDMessage::ReadDataResource(entity_id, data_resource_id, read_value_args) => {
                let reply = Self::select_entity(entities, &entity_id, |entity| {
                    entity.read_data_resource(&data_resource_id, read_value_args)
                });
                SOVDReply::ReadDataResource(reply)
            }

            SOVDMessage::ListOperations(_) => {
                let all_entities: Vec<Arc<Entity>> = entities
                    .lock()
                    .expect("mutex acquisition failed")
                    .values()
                    .cloned()
                    .collect();
                let mut all_ops = Vec::new();
                for entity in &all_entities {
                    all_ops.extend(entity.list_operations());
                }
                SOVDReply::ListOperations(Ok(all_ops))
            }

            SOVDMessage::GetOperationMetadata((entity_id, op_id)) => {
                let reply = Self::select_entity(entities, &entity_id, |entity| {
                    entity.get_operation_info(&op_id)
                });
                SOVDReply::GetOperationMetadata(reply)
            }

            SOVDMessage::ExecuteOperation((entity_id, op_id, timeout)) => {
                let reply = Self::select_entity(entities, &entity_id, |entity| {
                    entity.execute_operation(&op_id, timeout)
                });
                SOVDReply::ExecuteOperation(reply)
            }

            SOVDMessage::ExecuteOperationCapability((entity_id, op_id, exec_id, value)) => {
                let exec_control = Self::select_entity(entities, &entity_id, |entity| {
                    entity.get_execution_control(&op_id, &exec_id)
                });
                let reply = match exec_control {
                    Ok(exec_control) => {
                        let event = ExecutionEvent::from_kind(
                            ExecutionEventKind::HandleCustomCapability(value),
                        );
                        exec_control.send(event).await.map_err(|_| {
                            DiagError::from_error(sovd::GenericError::from_code(
                                sovd::ErrorCode::ErrorResponse,
                                "Execution is no longer active!".to_string(),
                            ))
                        })
                    }
                    Err(e) => Err(e),
                };
                SOVDReply::ExecuteOperationCapability(reply)
            }

            SOVDMessage::GetOperationExecutionStatus((entity_id, op_id, exec_id)) => {
                let exec_control = Self::select_entity(entities, &entity_id, |entity| {
                    entity.get_execution_control(&op_id, &exec_id)
                });
                let reply = match exec_control {
                    Ok(exec_control) => {
                        let (status_tx, status_rx) = oneshot::channel::<ExecutionStatus>();
                        let event = ExecutionEvent::from_kind(ExecutionEventKind::ReportStatus)
                            .with_status_reporter(move |status: ExecutionStatus, _| {
                                let _ = status_tx.send(status);
                            });
                        if exec_control.send(event).await.is_err() {
                            Err(DiagError::from_error(sovd::GenericError::from_code(
                                sovd::ErrorCode::ErrorResponse,
                                "Execution is no longer active!".to_string(),
                            )))
                        } else {
                            status_rx.await.map_err(|_| {
                                DiagError::from_error(sovd::GenericError::from_code(
                                    sovd::ErrorCode::SovdServerFailure,
                                    "Failed to receive execution status!".to_string(),
                                ))
                            })
                        }
                    }
                    Err(e) => Err(e),
                };
                SOVDReply::GetOperationExecutionStatus(reply)
            }

            SOVDMessage::GetOperationExecutionResult((entity_id, op_id, exec_id)) => {
                let reply = Self::select_entity(entities, &entity_id, |entity| {
                    entity.get_execution_result(&op_id, &exec_id)
                });
                SOVDReply::GetOperationExecutionResult(reply)
            }

            SOVDMessage::StopOperationExecution((entity_id, op_id, exec_id)) => {
                let exec_control = Self::select_entity(entities, &entity_id, |entity| {
                    entity.get_execution_control(&op_id, &exec_id)
                });
                let reply = match exec_control {
                    Ok(exec_control) => exec_control
                        .send(ExecutionEvent::from_kind(ExecutionEventKind::Stop))
                        .await
                        .map_err(|_| {
                            DiagError::from_error(sovd::GenericError::from_code(
                                sovd::ErrorCode::ErrorResponse,
                                "Execution is no longer active!".to_string(),
                            ))
                        }),
                    Err(e) => Err(e),
                };
                SOVDReply::StopOperationExecution(reply)
            }

            SOVDMessage::RemoveOperationExecution((entity_id, op_id, exec_id)) => {
                let reply = Self::select_entity(entities, &entity_id, |entity| {
                    entity.remove_execution(&op_id, &exec_id)
                });
                SOVDReply::RemoveOperationExecution(reply)
            }
        }
    }

    fn select_entity<T>(
        entities: &Mutex<IndexMap<EntityId, Arc<Entity>>>,
        entity_id: &EntityId,
        invoker: impl FnOnce(&Entity) -> DiagResult<T>,
    ) -> DiagResult<T> {
        match entities
            .lock()
            .map_err(|_| DiagError::mutex_error())?
            .get(entity_id)
        {
            Some(entity) => invoker(entity),
            None => Err(DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::ErrorResponse,
                format!("Entity with id '{}' could not be found!", entity_id),
            ))),
        }
    }

    // This method is just for demonstration purposes!
    pub fn send(&mut self, message: SOVDMessage) -> impl Future<Output = SOVDReply> {
        let (reply_sender, reply_receiver) = oneshot::channel::<SOVDReply>();
        let sovd_sender = self.sovd_sender.clone();
        async move {
            sovd_sender
                .send((message, reply_sender))
                .await
                .expect("sending SOVD message failed, channel is gone");
            reply_receiver
                .await
                .expect("receiving SOVD reply failed, channel is gone")
        }
    }

    pub fn get_or_create_entity(&self, id: EntityId) -> Arc<Entity> {
        self.entities
            .lock()
            .expect("mutex acquisition failed")
            .entry(id.clone())
            .or_insert(Arc::new(Entity::new(id)))
            .clone()
    }

    fn register_app_locally(
        &mut self,
        args: RegisterAppArgs,
        backend_reply: Option<RegisterAppReply>,
    ) -> DiagResult<RegisterAppReply> {
        if args.app_id.is_empty() {
            return Err(DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::IncompleteRequest,
                "app_id must not be empty".to_string(),
            )));
        }

        if args.endpoint.is_empty() {
            return Err(DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::IncompleteRequest,
                "endpoint must not be empty".to_string(),
            )));
        }

        self.get_or_create_entity(args.app_id.clone());

        self.registrations.insert(
            args.app_id,
            RegisteredApp {
                endpoint: args.endpoint,
                registration_id: backend_reply
                    .as_ref()
                    .and_then(|reply| reply.registration_id.clone()),
            },
        );

        self.registration_counter += 1;
        Ok(backend_reply.unwrap_or(RegisterAppReply {
            registration_id: Some(format!("reg-{}", self.registration_counter)),
            lease_ms: Some(DEFAULT_REGISTRATION_LEASE_MS),
        }))
    }

    fn deregister_app_locally(&mut self, args: DeregisterAppArgs) -> DiagResult<()> {
        if self.registrations.shift_remove(&args.app_id).is_none() {
            return Err(DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::ErrorResponse,
                format!("App with id '{}' is not registered", args.app_id),
            )));
        }

        self.entities
            .lock()
            .map_err(|_| DiagError::mutex_error())?
            .shift_remove(&args.app_id);

        Ok(())
    }

    fn registration_id_for(&self, app_id: &str) -> Option<String> {
        self.registrations
            .get(app_id)
            .and_then(|registered_app| registered_app.registration_id.clone())
    }

    fn resolve_endpoint(&self, app_id: &str) -> DiagResult<ReplyMessagePayload> {
        self.registrations
            .get(app_id)
            .map(|entry| ReplyMessagePayload::UTF8(entry.endpoint.clone()))
            .ok_or_else(|| {
                DiagError::from_error(sovd::GenericError::from_code(
                    sovd::ErrorCode::ErrorResponse,
                    format!("App with id '{}' is not registered", app_id),
                ))
            })
    }

    fn unregister_entity(&mut self, id: &EntityId) -> DiagResult<()> {
        let removed = self
            .entities
            .lock()
            .map_err(|_| DiagError::mutex_error())?
            .shift_remove(id);

        if removed.is_none() {
            return Err(DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::ErrorResponse,
                format!("Entity with id '{}' could not be found!", id),
            )));
        }

        self.registrations.shift_remove(id);
        Ok(())
    }

    pub fn shutdown(&mut self) -> impl Future<Output = ()> {
        let request_shutdown = self
            .request_shutdown
            .take()
            .expect("runtime got shut down already");
        async move {
            let _ = request_shutdown.send(());
        }
    }
}

pub struct Entity {
    #[allow(unused)] // since this is just example code
    data_resources: Mutex<IndexMap<DataResourceId, DataResourceHolder>>,
    operations: Mutex<IndexMap<OperationId, OperationHolder>>,
    id: EntityId,
}

impl Entity {
    pub fn new(id: EntityId) -> Self {
        Self {
            data_resources: Mutex::new(IndexMap::<DataResourceId, DataResourceHolder>::new()),
            operations: Mutex::new(IndexMap::<OperationId, OperationHolder>::new()),
            id: id,
        }
    }

    pub fn register_data_resource(
        &self,
        resource: impl DataResource + Send + 'static,
        resource_id: DataResourceId,
        resource_metadata: DataResourceMetadata,
    ) {
        match self
            .data_resources
            .lock()
            .expect("mutex acquisition failed")
            .insert(
                resource_id.clone(),
                DataResourceHolder::new(resource, resource_metadata),
            ) {
            Some(_) => panic!(
                "A data resource with id '{}' got already registered!",
                resource_id
            ),
            None => (),
        }
    }

    pub fn unregister_data_resource(&self, resource_id: &DataResourceId) -> DiagResult<()> {
        self.data_resources
            .lock()
            .map_err(|_| DiagError::mutex_error())?
            .shift_remove(resource_id)
            .map(|_| ())
            .ok_or_else(|| {
                DiagError::from_error(sovd::GenericError::from_code(
                    sovd::ErrorCode::ErrorResponse,
                    format!(
                        "Data resource with id '{}' not found in entity '{}'",
                        resource_id, self.id
                    ),
                ))
            })
    }

    pub fn register_operation(
        &self,
        op: impl Operation + Send + 'static,
        op_id: OperationId,
        op_metadata: OperationMetadata,
    ) {
        match self
            .operations
            .lock()
            .expect("mutex acquisition failed")
            .insert(op_id.clone(), OperationHolder::new(op, op_metadata))
        {
            Some(_) => panic!("An operation with id '{}' got already registered!", op_id),
            None => (),
        }
    }

    pub fn unregister_operation(&self, op_id: &OperationId) -> DiagResult<()> {
        let mut operations = self.operations.lock().map_err(|_| DiagError::mutex_error())?;
        let operation = operations.get(op_id).ok_or_else(|| {
            DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::ErrorResponse,
                format!(
                    "Operation with id '{}' not found in entity '{}'",
                    op_id, self.id
                ),
            ))
        })?;

        if !operation.executions.is_empty() {
            return Err(DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::PreconditionNotFulfilled,
                format!(
                    "Operation with id '{}' in entity '{}' still has executions and cannot be unregistered",
                    op_id, self.id
                ),
            )));
        }

        operations.shift_remove(op_id);
        Ok(())
    }

    pub fn list_data_resources(&self) -> Vec<DataResourceMetadata> {
        self.data_resources
            .lock()
            .expect("mutex acquisition failed")
            .values()
            .map(|resource| resource.metadata.clone())
            .collect()
    }

    pub fn read_data_resource(
        &self,
        id: &DataResourceId,
        args: ReadValueArgs,
    ) -> DiagResult<ReadValueReply> {
        self.data_resources
            .lock()
            .map_err(|_| DiagError::mutex_error())?
            .get(id)
            .ok_or_else(|| {
                DiagError::from_error(sovd::GenericError::from_code(
                    sovd::ErrorCode::ErrorResponse,
                    format!(
                        "Data resource with id '{}' not found in entity '{}'",
                        id, self.id
                    ),
                ))
            })?
            .instance
            .read(args)
    }

    pub fn list_operations(&self) -> Vec<OperationMetadata> {
        self.operations
            .lock()
            .expect("mutex acquisition failed")
            .values()
            .map(|operation| operation.info.clone())
            .collect()
    }

    pub fn get_operation_info(&self, op_id: &OperationId) -> DiagResult<OperationMetadata> {
        self.operations
            .lock()
            .map_err(|_| DiagError::mutex_error())?
            .get(op_id)
            .map(|operation| operation.info.clone())
            .ok_or_else(|| {
                DiagError::from_error(sovd::GenericError::from_code(
                    sovd::ErrorCode::ErrorResponse,
                    format!(
                        "Operation with id '{}' not found in entity '{}'",
                        op_id, self.id
                    ),
                ))
            })
    }

    pub fn execute_operation(
        &self,
        op_id: &OperationId,
        timeout: Option<ExecutionTimeout>,
    ) -> DiagResult<ExecutionId> {
        let mut operations = self.operations.lock().map_err(|_| DiagError::mutex_error())?;
        let operation = operations.get_mut(op_id).ok_or_else(|| {
            DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::ErrorResponse,
                format!(
                    "Operation with id '{}' not found in entity '{}'",
                    op_id, self.id
                ),
            ))
        })?;

        if operation.info.exclusive_execution && !operation.executions.is_empty() {
            return Err(DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::PreconditionNotFulfilled,
                format!(
                    "Operation with id '{}' requires exclusive invocation and already has an active execution!",
                    op_id
                ),
            )));
        }

        let (exec_event_sender, exec_event_receiver) = mpsc::channel(10);
        let exec_id = issue_new_execution_id();
        let exec_control = ExecutionControl::from(
            ExecutionControlImpl::new(exec_event_receiver),
            exec_id.clone(),
        );
        let exec_control_for_timeout = if timeout.is_some() {
            Some(exec_event_sender.clone())
        } else {
            None
        };

        let execution_handle = operation.instance.execute(
            ExecuteArguments {
                reply_encoding: ReplyMessageEncoding::UTF8, // UTF8 is used for demonstration purposes here
                user_parameters: None,
                additional_attrs: None,
                proximity_response: None,
            },
            exec_control,
        )?;

        let exec_future = execution_handle.future;
        let exec_future_with_timeout = async move {
            match timeout {
                Some(duration) => {
                    tokio::select! {
                        result = exec_future => result,
                        _ = tokio::time::sleep(duration) => {
                            let _ = exec_control_for_timeout.expect("unexpected None value").send(ExecutionEvent::from_kind(ExecutionEventKind::Stop)).await;
                            Err(DiagError::from_error(sovd::GenericError::from_code(
                                sovd::ErrorCode::ErrorResponse,
                                "execution got stopped due to timeout".to_string(),
                            )))
                        }
                    }
                }
                None => exec_future.await,
            }
        };

        if operation.info.synchronous_execution {
            operation.executions.insert(
                exec_id.clone(),
                ActiveExecution::Running {
                    exec_control: exec_event_sender,
                    join_handle: None,
                },
            );
            drop(operations); // for unlocking the mutex
            let result = futures::executor::block_on(exec_future_with_timeout);
            let mut operations = self.operations.lock().map_err(|_| DiagError::mutex_error())?;
            let operation = operations.get_mut(op_id).expect("operation must exist");
            if let Some(execution) = operation.executions.get_mut(&exec_id) {
                *execution = ActiveExecution::Completed(result);
            }
        } else {
            operation.executions.insert(
                exec_id.clone(),
                ActiveExecution::Running {
                    exec_control: exec_event_sender,
                    join_handle: Some(tokio::spawn(exec_future_with_timeout)),
                },
            );
        };

        Ok(exec_id)
    }

    pub fn get_execution_control(
        &self,
        op_id: &OperationId,
        exec_id: &ExecutionId,
    ) -> DiagResult<mpsc::Sender<ExecutionEvent>> {
        let operations = self.operations.lock().map_err(|_| DiagError::mutex_error())?;
        let operation = operations.get(op_id).ok_or_else(|| {
            DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::ErrorResponse,
                format!(
                    "Operation with id '{}' not found in entity '{}'",
                    op_id, self.id
                ),
            ))
        })?;
        let execution = operation.executions.get(exec_id).ok_or_else(|| {
            DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::ErrorResponse,
                format!(
                    "Execution with id '{}' not found for operation '{}' in entity '{}'",
                    exec_id, op_id, self.id
                ),
            ))
        })?;
        execution.get_exec_control()
    }

    pub fn get_execution_result(
        &self,
        op_id: &OperationId,
        exec_id: &ExecutionId,
    ) -> DiagResult<ExecutionResult> {
        let mut operations = self.operations.lock().map_err(|_| DiagError::mutex_error())?;
        let operation = operations.get_mut(op_id).ok_or_else(|| {
            DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::ErrorResponse,
                format!(
                    "Operation with id '{}' not found in entity '{}'",
                    op_id, self.id
                ),
            ))
        })?;
        let execution = operation.executions.get_mut(exec_id).ok_or_else(|| {
            DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::ErrorResponse,
                format!(
                    "Execution with id '{}' not found for operation '{}' in entity '{}'",
                    exec_id, op_id, self.id
                ),
            ))
        })?;
        execution.try_resolve();
        execution.get_result()
    }

    pub fn remove_execution(&self, op_id: &OperationId, exec_id: &ExecutionId) -> DiagResult<()> {
        let mut operations = self.operations.lock().map_err(|_| DiagError::mutex_error())?;
        let operation = operations.get_mut(op_id).ok_or_else(|| {
            DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::ErrorResponse,
                format!(
                    "Operation with id '{}' not found in entity '{}'",
                    op_id, self.id
                ),
            ))
        })?;
        let execution = operation.executions.get_mut(exec_id).ok_or_else(|| {
            DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::ErrorResponse,
                format!(
                    "Execution with id '{}' not found for operation '{}' in entity '{}'",
                    exec_id, op_id, self.id
                ),
            ))
        })?;
        execution.try_resolve();
        if matches!(execution, ActiveExecution::Running { .. }) {
            return Err(DiagError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::PreconditionNotFulfilled,
                format!(
                    "Execution with id '{}' for operation '{}' in entity '{}' is still running and must explicitly get stopped prior to removal!",
                    exec_id, op_id, self.id
                ),
            )));
        }
        operation.executions.shift_remove(exec_id);
        Ok(())
    }
}

struct DataResourceHolder {
    metadata: DataResourceMetadata,
    instance: Box<dyn DataResource + Send>,
}

impl DataResourceHolder {
    fn new(
        resource: impl DataResource + Send + 'static,
        resource_metadata: DataResourceMetadata,
    ) -> Self {
        Self {
            metadata: resource_metadata,
            instance: Box::new(resource),
        }
    }
}

struct OperationHolder {
    info: OperationMetadata,
    instance: Box<dyn Operation + Send>,
    executions: IndexMap<ExecutionId, ActiveExecution>,
}

impl OperationHolder {
    fn new(op: impl Operation + Send + 'static, op_metadata: OperationMetadata) -> Self {
        Self {
            info: op_metadata,
            instance: Box::new(op),
            executions: IndexMap::<ExecutionId, ActiveExecution>::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex as StdMutex;

    use diag_api::sovd::data_resource::DataCategory;
    use diag_api::sovd::operation::ExecutionHandle;

    struct ReadOnlyResource;

    impl DataResource for ReadOnlyResource {
        fn read(&self, _args: ReadValueArgs) -> DiagResult<ReadValueReply> {
            Ok(ReadValueReply {
                id: Some("resource-1".to_string()),
                data: ReplyMessagePayload::UTF8("value".to_string()),
                errors: None,
            })
        }
    }

    struct NoOpOperation;

    impl Operation for NoOpOperation {
        fn execute(
            &mut self,
            _input: ExecuteArguments,
            _control: ExecutionControl,
        ) -> DiagResult<ExecutionHandle> {
            ExecutionHandle::from_closure(|| Ok(DiagnosticReply::default()))
        }
    }

    #[derive(Default)]
    struct RecordingRegistrar {
        registered: StdMutex<Vec<RegisterAppArgs>>,
        deregistered: StdMutex<Vec<DeregisterAppArgs>>,
    }

    impl AppRegistrar for RecordingRegistrar {
        fn register_app(
            &self,
            args: RegisterAppArgs,
        ) -> BoxFuture<'_, DiagResult<RegisterAppReply>> {
            self.registered
                .lock()
                .expect("lock should succeed")
                .push(args);
            async move {
                Ok(RegisterAppReply {
                    registration_id: Some("backend-reg-1".to_string()),
                    lease_ms: Some(45_000),
                })
            }
            .boxed()
        }

        fn deregister_app(&self, args: DeregisterAppArgs) -> BoxFuture<'_, DiagResult<()>> {
            self.deregistered
                .lock()
                .expect("lock should succeed")
                .push(args);
            async move { Ok(()) }.boxed()
        }
    }

    #[tokio::test]
    async fn runtime_register_and_resolve_endpoint() {
        let runtime = Runtime::new();

        let reply = runtime
            .register_app(RegisterAppArgs {
                app_id: "APP01".to_string(),
                app_name: "Diagnostics App".to_string(),
                hosted_on: "HPC".to_string(),
                endpoint: "http://127.0.0.1:8081/api".to_string(),
                additional_attrs: None,
            })
            .await
            .expect("registration should succeed");

        assert_eq!(reply.lease_ms, Some(DEFAULT_REGISTRATION_LEASE_MS));
        assert!(reply.registration_id.is_some());

        let endpoint = runtime
            .resolve_endpoint("APP01")
            .await
            .expect("resolve should succeed");
        assert_eq!(
            endpoint,
            ReplyMessagePayload::UTF8("http://127.0.0.1:8081/api".to_string())
        );
    }

    #[tokio::test]
    async fn runtime_deregister_removes_registration() {
        let runtime = Runtime::new();

        runtime
            .register_app(RegisterAppArgs {
                app_id: "APP02".to_string(),
                app_name: "Telemetry App".to_string(),
                hosted_on: "HPC".to_string(),
                endpoint: "http://127.0.0.1:8082/api".to_string(),
                additional_attrs: None,
            })
            .await
            .expect("registration should succeed");

        runtime
            .deregister_app(DeregisterAppArgs {
                app_id: "APP02".to_string(),
                registration_id: None,
            })
            .await
            .expect("deregistration should succeed");

        let err = runtime
            .resolve_endpoint("APP02")
            .await
            .expect_err("resolve should fail after deregistration");

        match err.code {
            ErrorCode::SOVD(inner) => {
                assert_eq!(inner.sovd_error, sovd::ErrorCode::ErrorResponse.to_string());
            }
            _ => panic!("expected SOVD error code"),
        }
    }

    #[tokio::test]
    async fn runtime_register_app_forwards_to_open_sovd_backend() {
        let registrar = Arc::new(RecordingRegistrar::default());
        let runtime = Runtime::with_registrar_backend(registrar.clone());

        let reply = runtime
            .register_app(RegisterAppArgs {
                app_id: "APP03".to_string(),
                app_name: "Proxy App".to_string(),
                hosted_on: "HPC".to_string(),
                endpoint: "http://127.0.0.1:8083/api".to_string(),
                additional_attrs: None,
            })
            .await
            .expect("registration should succeed");

        let registered = registrar.registered.lock().expect("lock should succeed");
        assert_eq!(registered.len(), 1);
        assert_eq!(registered[0].app_id, "APP03");
        assert_eq!(reply.registration_id, Some("backend-reg-1".to_string()));
        assert_eq!(reply.lease_ms, Some(45_000));
    }

    #[tokio::test]
    async fn runtime_deregister_app_forwards_to_open_sovd_backend() {
        let registrar = Arc::new(RecordingRegistrar::default());
        let runtime = Runtime::with_registrar_backend(registrar.clone());

        runtime
            .register_app(RegisterAppArgs {
                app_id: "APP04".to_string(),
                app_name: "Proxy App".to_string(),
                hosted_on: "HPC".to_string(),
                endpoint: "http://127.0.0.1:8084/api".to_string(),
                additional_attrs: None,
            })
            .await
            .expect("registration should succeed");

        runtime
            .deregister_app(DeregisterAppArgs {
                app_id: "APP04".to_string(),
                registration_id: None,
            })
            .await
            .expect("deregistration should succeed");

        let deregistered = registrar.deregistered.lock().expect("lock should succeed");
        assert_eq!(deregistered.len(), 1);
        assert_eq!(deregistered[0].app_id, "APP04");
        assert_eq!(deregistered[0].registration_id, Some("backend-reg-1".to_string()));
    }

    #[test]
    fn runtime_unregister_entity_removes_entity_and_registration() {
        let runtime = Runtime::new();

        futures::executor::block_on(runtime.register_app(RegisterAppArgs {
            app_id: "APP_UNREGISTER".to_string(),
            app_name: "Unregister App".to_string(),
            hosted_on: "HPC".to_string(),
            endpoint: "http://127.0.0.1:8086/api".to_string(),
            additional_attrs: None,
        }))
        .expect("registration should succeed");

        runtime
            .unregister_entity(&"APP_UNREGISTER".to_string())
            .expect("entity unregister should succeed");

        let err = futures::executor::block_on(runtime.resolve_endpoint("APP_UNREGISTER"))
            .expect_err("resolve should fail after unregister");

        match err.code {
            ErrorCode::SOVD(inner) => {
                assert_eq!(inner.sovd_error, sovd::ErrorCode::ErrorResponse.to_string());
            }
            _ => panic!("expected SOVD error code"),
        }
    }

    #[test]
    fn entity_unregister_data_resource_removes_resource() {
        let entity = Entity::new("ENTITY1".to_string());
        let resource_id = "resource-1".to_string();

        entity.register_data_resource(
            ReadOnlyResource,
            resource_id.clone(),
            DataResourceMetadata {
                id: resource_id.clone(),
                name: "Resource 1".to_string(),
                translation_id: None,
                category: DataCategory::CurrentData,
                groups: None,
            },
        );

        entity
            .unregister_data_resource(&resource_id)
            .expect("resource unregister should succeed");

        let err = entity
            .read_data_resource(&resource_id, ReadValueArgs::from(ReplyMessageEncoding::UTF8))
            .expect_err("read should fail after unregister");

        match err.code {
            ErrorCode::SOVD(inner) => {
                assert_eq!(inner.sovd_error, sovd::ErrorCode::ErrorResponse.to_string());
            }
            _ => panic!("expected SOVD error code"),
        }
    }

    #[test]
    fn entity_unregister_operation_removes_operation() {
        let entity = Entity::new("ENTITY2".to_string());
        let op_id = "op-1".to_string();

        entity.register_operation(
            NoOpOperation,
            op_id.clone(),
            OperationMetadata {
                proximity_proof_required: false,
                synchronous_execution: false,
                exclusive_execution: false,
                supported_modes: None,
            },
        );

        entity
            .unregister_operation(&op_id)
            .expect("operation unregister should succeed");

        let err = entity
            .get_operation_info(&op_id)
            .expect_err("lookup should fail after unregister");

        match err.code {
            ErrorCode::SOVD(inner) => {
                assert_eq!(inner.sovd_error, sovd::ErrorCode::ErrorResponse.to_string());
            }
            _ => panic!("expected SOVD error code"),
        }
    }
}
