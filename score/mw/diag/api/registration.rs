// *******************************************************************************
// Copyright (c) 2026 Contributors to the Eclipse Foundation
//
// See the NOTICE file(s) distributed with this work for additional
// information regarding copyright ownership.
//
// This program and the accompanying materials are made available under the
// terms of the Apache License Version 2.0 which is available at
// <https://www.apache.org/licenses/LICENSE-2.0>
//
// SPDX-License-Identifier: Apache-2.0
// *******************************************************************************

use indexmap::{IndexMap, IndexSet};

use ::common::{Error, JsonSchema, Result};

use ::data_resource::sovd::DataResourceMetadata;
use ::data_resource::{DataResource, ReadOnlyDataResource, WritableDataResource};
use ::operation::{Operation, OperationMetadata};
use ::uds::{DataIdentifier, ReadDataByIdentifier, RoutineControl, WriteDataByIdentifier};

// ============================================================================
// SOVD Builder Pattern
// ============================================================================

/// Trait that diagnostic entities must implement to be registered with SOVD services
pub trait DiagnosticEntity: Send + Sync {
    /// Returns a unique identifier for this diagnostic entity
    fn entity_id(&self) -> &str;
}

/// SOVD Service Collection Builder with fluent API for assembling an entity's services
///
/// Each data-resource method takes the instance, its [`DataResourceMetadata`]
/// (which carries the resource `id`), and a JSON schema describing the payload.
/// Each operation method takes a separate `id` string plus [`OperationMetadata`].
pub struct DiagnosticServicesCollectionBuilder<'entity> {
    entity: Box<dyn DiagnosticEntity + Send + Sync + 'entity>,

    // Metadata stored separately from instances so the collection can expose
    // clean &IndexMap<String, (DataResourceMetadata, JsonSchema)> without any
    // Box<dyn Trait> leaking into the public API.
    read_resource_metadata:  IndexMap<String, (DataResourceMetadata, JsonSchema)>,
    write_resource_metadata: IndexMap<String, (DataResourceMetadata, JsonSchema)>,
    data_resource_metadata:  IndexMap<String, (DataResourceMetadata, JsonSchema)>,
    operation_metadata:      IndexMap<String, OperationMetadata>,

    read_resource_instances:  IndexMap<String, Box<dyn ReadOnlyDataResource  + Send + Sync + 'static>>,
    write_resource_instances: IndexMap<String, Box<dyn WritableDataResource  + Send + Sync + 'static>>,
    data_resource_instances:  IndexMap<String, Box<dyn DataResource          + Send + Sync + 'static>>,
    operation_instances:      IndexMap<String, Box<dyn Operation             + Send + Sync + 'static>>,
}

impl<'entity> DiagnosticServicesCollectionBuilder<'entity> {
    /// Creates a new SOVD services builder, taking ownership of the entity.
    pub fn new<T: DiagnosticEntity + Send + Sync + 'entity>(entity: T) -> Self {
        Self {
            entity: Box::new(entity),
            read_resource_metadata:  IndexMap::new(),
            write_resource_metadata: IndexMap::new(),
            data_resource_metadata:  IndexMap::new(),
            operation_metadata:      IndexMap::new(),
            read_resource_instances:  IndexMap::new(),
            write_resource_instances: IndexMap::new(),
            data_resource_instances:  IndexMap::new(),
            operation_instances:      IndexMap::new(),
        }
    }

    /// Registers a read-only data resource.  The resource ID is taken from `metadata.id`.
    pub fn with_read_resource<T: ReadOnlyDataResource + Send + Sync + 'static>(
        mut self,
        resource: T,
        metadata: DataResourceMetadata,
        schema: JsonSchema,
    ) -> Self {
        let id = metadata.id.clone();
        self.read_resource_instances.insert(id.clone(), Box::new(resource));
        self.read_resource_metadata.insert(id, (metadata, schema));
        self
    }

    /// Registers a write-only data resource.  The resource ID is taken from `metadata.id`.
    pub fn with_write_resource<T: WritableDataResource + Send + Sync + 'static>(
        mut self,
        resource: T,
        metadata: DataResourceMetadata,
        schema: JsonSchema,
    ) -> Self {
        let id = metadata.id.clone();
        self.write_resource_instances.insert(id.clone(), Box::new(resource));
        self.write_resource_metadata.insert(id, (metadata, schema));
        self
    }

    /// Registers a read-write data resource.  The resource ID is taken from `metadata.id`.
    pub fn with_data_resource<T: DataResource + Send + Sync + 'static>(
        mut self,
        resource: T,
        metadata: DataResourceMetadata,
        schema: JsonSchema,
    ) -> Self {
        let id = metadata.id.clone();
        self.data_resource_instances.insert(id.clone(), Box::new(resource));
        self.data_resource_metadata.insert(id, (metadata, schema));
        self
    }

    /// Registers an operation
    ///
    /// `id` identifies the operation; `metadata` carries its runtime-visible attributes.
    pub fn with_operation<T: Operation + Send + Sync + 'static>(
        mut self,
        operation: T,
        id: impl Into<String>,
        metadata: OperationMetadata,
    ) -> Self {
        let id = id.into();
        self.operation_instances.insert(id.clone(), Box::new(operation));
        self.operation_metadata.insert(id, metadata);
        self
    }

    /// Builds and returns the final DiagnosticServicesCollection
    pub fn build(self) -> Result<DiagnosticServicesCollection<'entity>> {
        self.validate()?;
        Ok(DiagnosticServicesCollection {
            entity: self.entity,
            read_resource_metadata:  self.read_resource_metadata,
            write_resource_metadata: self.write_resource_metadata,
            data_resource_metadata:  self.data_resource_metadata,
            operation_metadata:      self.operation_metadata,
            read_resource_instances:  self.read_resource_instances,
            write_resource_instances: self.write_resource_instances,
            data_resource_instances:  self.data_resource_instances,
            operation_instances:      self.operation_instances,
        })
    }

    fn validate(&self) -> Result<()> {
        let all_ids: Vec<_> = self
            .read_resource_metadata.keys()
            .chain(self.write_resource_metadata.keys())
            .chain(self.data_resource_metadata.keys())
            .chain(self.operation_metadata.keys())
            .collect();
        let all_ids_len = all_ids.len();
        let unique: IndexSet<_> = all_ids.into_iter().collect();
        if unique.len() != all_ids_len {
            return Err(Error::from_error(::common::sovd::GenericError::from_code(
                ::common::sovd::ErrorCode::SovdServerMisconfigured,
                "Duplicate service ID detected".to_string(),
            )));
        }
        Ok(())
    }
}

// ============================================================================
// SOVD Services Collection
// ============================================================================

/// Collection produced by [`DiagnosticServicesCollectionBuilder`].
///
/// Storage is split into two layers:
/// - **Metadata maps** (`read_resources()`, `data_resources()`, …) — clean
///   `&IndexMap<String, (DataResourceMetadata, JsonSchema)>` with no dyn traits;
///   callers can use the full IndexMap API (`.len()`, `.contains_key()`,
///   `.iter()`, `.get()`, etc.).
/// - **Instance getters** (`get_read_resource(id)`, …) — encapsulate the
///   `Box<dyn Trait>` so it never appears in any public signature.
///
/// The [`ServiceRegistrar`] iterates the metadata map to populate the runtime
/// and calls instance getters when it needs to dispatch to the actual implementation.
pub struct DiagnosticServicesCollection<'entity> {
    entity: Box<dyn DiagnosticEntity + Send + Sync + 'entity>,

    // Public metadata maps — no dyn traits
    read_resource_metadata:  IndexMap<String, (DataResourceMetadata, JsonSchema)>,
    write_resource_metadata: IndexMap<String, (DataResourceMetadata, JsonSchema)>,
    data_resource_metadata:  IndexMap<String, (DataResourceMetadata, JsonSchema)>,
    operation_metadata:      IndexMap<String, OperationMetadata>,

    // Private instance maps — accessed only through per-item getters
    read_resource_instances:  IndexMap<String, Box<dyn ReadOnlyDataResource  + Send + Sync + 'static>>,
    write_resource_instances: IndexMap<String, Box<dyn WritableDataResource  + Send + Sync + 'static>>,
    data_resource_instances:  IndexMap<String, Box<dyn DataResource          + Send + Sync + 'static>>,
    operation_instances:      IndexMap<String, Box<dyn Operation             + Send + Sync + 'static>>,
}

impl<'entity> DiagnosticServicesCollection<'entity> {
    /// Returns reference to the registered entity
    pub fn entity(&self) -> &(dyn DiagnosticEntity + Send + Sync) {
        &*self.entity
    }

    /// Returns all registered read resources
    pub fn read_resources(&self) -> &IndexMap<String, (DataResourceMetadata, JsonSchema)> {
        &self.read_resource_metadata
    }

    /// Returns all registered write resources
    pub fn write_resources(&self) -> &IndexMap<String, (DataResourceMetadata, JsonSchema)> {
        &self.write_resource_metadata
    }

    /// Returns all registered read-write data resource
    pub fn data_resources(&self) -> &IndexMap<String, (DataResourceMetadata, JsonSchema)> {
        &self.data_resource_metadata
    }

    /// Returns all registered operations
    pub fn operations(&self) -> &IndexMap<String, OperationMetadata> {
        &self.operation_metadata
    }

    /// Consuming extractors to pass real instance ownership to the registrar layer

    pub fn into_read_resources(self) -> IndexMap<String, Box<dyn ReadOnlyDataResource + Send + Sync + 'static>> {
        self.read_resource_instances
    }

    pub fn into_write_resources(self) -> IndexMap<String, Box<dyn WritableDataResource + Send + Sync + 'static>> {
        self.write_resource_instances
    }

    pub fn into_data_resources(self) -> IndexMap<String, Box<dyn DataResource + Send + Sync + 'static>> {
        self.data_resource_instances
    }

    pub fn into_operations(self) -> IndexMap<String, Box<dyn Operation + Send + Sync + 'static>> {
        self.operation_instances
    }

    // ------------------------------------------------------------------
    // Per-item instance getters — encapsulate Box<dyn Trait>.
    // Use the metadata maps above to iterate registered services;
    // use these getters when the runtime needs to invoke the instance.
    // ------------------------------------------------------------------

    /// Returns the read-only resource instance for the given id.
    pub fn get_read_resource(&self, id: &str) -> Option<&dyn ReadOnlyDataResource> {
        self.read_resource_instances.get(id).map(|r| r.as_ref() as &dyn ReadOnlyDataResource)
    }

    /// Returns a mutable reference to the write resource instance for the given id.
    pub fn get_write_resource_mut(&mut self, id: &str) -> Option<&mut dyn WritableDataResource> {
        self.write_resource_instances.get_mut(id).map(|r| r.as_mut() as &mut dyn WritableDataResource)
    }

    /// Returns the data resource instance for the given id.
    pub fn get_data_resource(&self, id: &str) -> Option<&dyn DataResource> {
        self.data_resource_instances.get(id).map(|r| r.as_ref() as &dyn DataResource)
    }

    /// Returns the operation instance for the given id.
    pub fn get_operation(&self, id: &str) -> Option<&dyn Operation> {
        self.operation_instances.get(id).map(|r| r.as_ref() as &dyn Operation)
    }

    /// Returns a mutable reference to the operation instance for the given id.
    pub fn get_operation_mut(&mut self, id: &str) -> Option<&mut dyn Operation> {
        self.operation_instances.get_mut(id).map(|r| r.as_mut() as &mut dyn Operation)
    }
}

// ============================================================================
// Registration Handles — RAII lifetime guards
// ============================================================================

/// RAII handle for diagnostic service registration (SOVD or UDS).
///
/// The application **must** keep this handle alive for as long as the registered
/// services should remain active.  Dropping the handle automatically signals
/// the runtime to deregister all associated services — no manual cleanup needed.
///
/// Both [`ServiceRegistrar::register_sovd_services`] and
/// [`ServiceRegistrar::register_uds_services`] return this same type.
#[must_use = "dropping this handle immediately deregisters all associated services"]
pub struct RegistrationHandle {
    deregister: Option<Box<dyn FnOnce() + Send>>,
}

impl RegistrationHandle {
    /// Creates a new handle with a deregistration callback.
    /// Called by the runtime implementation — not by application code.
    #[must_use = "dropping this handle immediately deregisters all associated services"]
    pub fn new(deregister: impl FnOnce() + Send + 'static) -> Self {
        Self { deregister: Some(Box::new(deregister)) }
    }
}

impl Drop for RegistrationHandle {
    fn drop(&mut self) {
        if let Some(f) = self.deregister.take() {
            f();
        }
    }
}

// ============================================================================
// Service Registration Binding Layer
// ============================================================================

/// Binding layer between user-built service collections and the runtime.
/// User code obtains a [`ServiceRegistrar`] from the runtime and passes collections to it.
/// The runtime implements this trait internally; user code never touches the runtime directly.
///
/// Both registration methods return a [`RegistrationHandle`].  The application
/// must store the handle — dropping it automatically deregisters all associated
/// services.
pub trait ServiceRegistrar {
    /// Registers a SOVD service collection (entity + data resources + operations).
    /// Returns a [`RegistrationHandle`] that keeps services registered for its lifetime.
    fn register_sovd_services<'entity>(
        &self,
        collection: DiagnosticServicesCollection<'entity>,
    ) -> Result<RegistrationHandle>;

    /// Registers a UDS service collection (DIDs + routines).
    /// Returns a [`RegistrationHandle`] that keeps services registered for its lifetime.
    fn register_uds_services(&self, collection: UdsServicesCollection) -> Result<RegistrationHandle>;
}

// ============================================================================
// UDS Builder Pattern
// ============================================================================

/// Entity-agnostic builder for UDS diagnostic services (DIDs and routines)
#[derive(Default)]
pub struct UdsServicesCollectionBuilder {
    data_ids:   IndexMap<String, Box<dyn DataIdentifier           + Send + Sync + 'static>>,
    read_dids:  IndexMap<String, Box<dyn ReadDataByIdentifier     + Send + Sync + 'static>>,
    write_dids: IndexMap<String, Box<dyn WriteDataByIdentifier    + Send + Sync + 'static>>,
    routines:   IndexMap<String, Box<dyn RoutineControl           + Send + Sync + 'static>>,
}

impl UdsServicesCollectionBuilder {
    /// Creates a new UDS services builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a combined read+write Data Identifier.
    pub fn with_data_id<T: DataIdentifier + Send + Sync + 'static>(mut self, id: impl Into<String>, service: T) -> Self {
        self.data_ids.insert(id.into(), Box::new(service));
        self
    }

    /// Registers a read Data Identifier (DID)
    pub fn with_read_did<T: ReadDataByIdentifier + Send + Sync + 'static>(mut self, id: impl Into<String>, service: T) -> Self {
        self.read_dids.insert(id.into(), Box::new(service));
        self
    }

    /// Registers a write Data Identifier (DID)
    pub fn with_write_did<T: WriteDataByIdentifier + Send + Sync + 'static>(mut self, id: impl Into<String>, service: T) -> Self {
        self.write_dids.insert(id.into(), Box::new(service));
        self
    }

    /// Registers a routine control handler
    pub fn with_routine<T: RoutineControl + Send + Sync + 'static>(mut self, id: impl Into<String>, routine: T) -> Self {
        self.routines.insert(id.into(), Box::new(routine));
        self
    }

    /// Builds and returns the final UdsServicesCollection
    pub fn build(self) -> Result<UdsServicesCollection> {
        self.validate()?;
        // Derive ID sets from the instance maps — no duplicate storage needed in the builder.
        let data_id_keys:   IndexSet<String> = self.data_ids.keys().cloned().collect();
        let read_did_keys:  IndexSet<String> = self.read_dids.keys().cloned().collect();
        let write_did_keys: IndexSet<String> = self.write_dids.keys().cloned().collect();
        let routine_keys:   IndexSet<String> = self.routines.keys().cloned().collect();
        Ok(UdsServicesCollection {
            data_id_keys,
            read_did_keys,
            write_did_keys,
            routine_keys,
            data_ids:   self.data_ids,
            read_dids:  self.read_dids,
            write_dids: self.write_dids,
            routines:   self.routines,
        })
    }

    fn validate(&self) -> Result<()> {
        let all_ids: Vec<_> = self
            .data_ids.keys()
            .chain(self.read_dids.keys())
            .chain(self.write_dids.keys())
            .chain(self.routines.keys())
            .collect();
        let all_ids_len = all_ids.len();
        let unique: IndexSet<_> = all_ids.into_iter().collect();
        if unique.len() != all_ids_len {
            return Err(Error::from_error(::common::sovd::GenericError::from_code(
                ::common::sovd::ErrorCode::SovdServerMisconfigured,
                "Duplicate service ID detected".to_string(),
            )));
        }
        Ok(())
    }
}

/// UDS Services Collection from builder pattern
pub struct UdsServicesCollection {
    // ID sets — full IndexSet API (len, contains, iter, is_empty) with no dyn traits
    data_id_keys:   IndexSet<String>,
    read_did_keys:  IndexSet<String>,
    write_did_keys: IndexSet<String>,
    routine_keys:   IndexSet<String>,

    // Instance maps — accessed only through per-item getters
    data_ids:   IndexMap<String, Box<dyn DataIdentifier           + Send + Sync + 'static>>,
    read_dids:  IndexMap<String, Box<dyn ReadDataByIdentifier     + Send + Sync + 'static>>,
    write_dids: IndexMap<String, Box<dyn WriteDataByIdentifier    + Send + Sync + 'static>>,
    routines:   IndexMap<String, Box<dyn RoutineControl           + Send + Sync + 'static>>,
}

impl UdsServicesCollection {
    /// Consuming extractors to pass real instance ownership to the registrar layer

    pub fn into_data_ids(self) -> IndexMap<String, Box<dyn DataIdentifier + Send + Sync + 'static>> {
        self.data_ids
    }

    pub fn into_read_dids(self) -> IndexMap<String, Box<dyn ReadDataByIdentifier + Send + Sync + 'static>> {
        self.read_dids
    }

    pub fn into_write_dids(self) -> IndexMap<String, Box<dyn WriteDataByIdentifier + Send + Sync + 'static>> {
        self.write_dids
    }

    pub fn into_routines(self) -> IndexMap<String, Box<dyn RoutineControl + Send + Sync + 'static>> {
        self.routines
    }

    /// Returns the combined read+write DID instance for the given id.
    pub fn get_data_id(&self, id: &str) -> Option<&dyn DataIdentifier> {
        self.data_ids.get(id).map(|r| r.as_ref() as &dyn DataIdentifier)
    }

    /// Returns the read DID instance for the given id.
    pub fn get_read_did(&self, id: &str) -> Option<&dyn ReadDataByIdentifier> {
        self.read_dids.get(id).map(|r| r.as_ref() as &dyn ReadDataByIdentifier)
    }

    /// Returns a mutable reference to the write DID instance for the given id.
    pub fn get_write_did_mut(&mut self, id: &str) -> Option<&mut dyn WriteDataByIdentifier> {
        self.write_dids.get_mut(id).map(|r| r.as_mut() as &mut dyn WriteDataByIdentifier)
    }

    /// Returns the routine instance for the given id.
    pub fn get_routine(&self, id: &str) -> Option<&dyn RoutineControl> {
        self.routines.get(id).map(|r| r.as_ref() as &dyn RoutineControl)
    }

    /// Returns a mutable reference to the routine instance for the given id.
    pub fn get_routine_mut(&mut self, id: &str) -> Option<&mut dyn RoutineControl> {
        self.routines.get_mut(id).map(|r| r.as_mut() as &mut dyn RoutineControl)
    }

    // ------------------------------------------------------------------
    // ID sets — full IndexSet API available to callers (len, contains,
    // iter, is_empty, …).  No Box<dyn Trait> in any returned type.
    // ------------------------------------------------------------------

    /// Returns the set of registered combined read+write DID ids.
    pub fn data_ids(&self) -> &IndexSet<String> { &self.data_id_keys }

    /// Returns the set of registered read DID ids.
    pub fn read_dids(&self) -> &IndexSet<String> { &self.read_did_keys }

    /// Returns the set of registered write DID ids.
    pub fn write_dids(&self) -> &IndexSet<String> { &self.write_did_keys }

    /// Returns the set of registered routine ids.
    pub fn routines(&self) -> &IndexSet<String> { &self.routine_keys }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::data_resource::sovd::DataCategory;
    use ::data_resource::{ReadValueArgs, ReadValueHandle, ReadValueReply, WriteValueArgs, WriteValueHandle};
    use ::operation::{ExecuteArguments, ExecutionControl, ExecutionHandle};

    // -----------------------------------------------------------------------
    // Minimal stubs
    // -----------------------------------------------------------------------

    struct TestEntity { id: String }
    impl DiagnosticEntity for TestEntity {
        fn entity_id(&self) -> &str { &self.id }
    }

    struct StubReadResource;
    impl ::data_resource::ReadOnlyDataResource for StubReadResource {
        fn read(&self, _: ReadValueArgs) -> ReadValueHandle {
            ReadValueHandle::ready(ReadValueReply { data: ::common::ReplyMessagePayload::UTF8("ok".into()), errors: None })
        }
    }

    struct StubDataResource;
    impl ::data_resource::DataResource for StubDataResource {
        fn read(&self, _: ReadValueArgs) -> ReadValueHandle {
            ReadValueHandle::ready(ReadValueReply { data: ::common::ReplyMessagePayload::UTF8("ok".into()), errors: None })
        }
    }

    struct StubOperation;
    impl ::operation::Operation for StubOperation {
        fn execute(&mut self, _: ExecuteArguments, _: ExecutionControl) -> ::common::Result<ExecutionHandle> {
            Ok(ExecutionHandle::from_closure(|| Ok(::common::DiagnosticReply::default())))
        }
    }

    struct StubWritableDataResource;
    impl ::data_resource::WritableDataResource for StubWritableDataResource {
        fn write(&mut self, _: WriteValueArgs) -> WriteValueHandle {
            WriteValueHandle::ready(Ok(()))
        }
    }

    struct StubDataId;
    impl ::uds::ReadDataByIdentifier for StubDataId {
        fn read(&self) -> ::common::Result<Vec<u8>> { Ok(vec![0x00]) }
    }
    impl ::uds::WriteDataByIdentifier for StubDataId {
        fn write(&mut self, _: &[u8]) -> ::common::Result<()> { Ok(()) }
    }
    impl ::uds::DataIdentifier for StubDataId {}

    struct StubRdbi;
    impl ::uds::ReadDataByIdentifier for StubRdbi {
        fn read(&self) -> ::common::Result<Vec<u8>> { Ok(vec![]) }
    }

    struct StubWdbi;
    impl ::uds::WriteDataByIdentifier for StubWdbi {
        fn write(&mut self, _: &[u8]) -> ::common::Result<()> { Ok(()) }
    }

    struct StubRoutine;
    impl ::uds::RoutineControl for StubRoutine {
        fn start(&mut self, _: Option<&[u8]>) -> ::common::Result<::uds::StartRoutine> {
            ::uds::StartRoutine::from_closure(|| Ok(None), None)
        }
        fn stop(&mut self, _: Option<&[u8]>) -> ::common::Result<Option<Vec<u8>>> { Ok(None) }
        fn completion_percentage(&self) -> Option<u8> { None }
    }

    fn stub_metadata(id: &str) -> DataResourceMetadata {
        DataResourceMetadata {
            id: id.to_string(), name: id.to_string(), translation_id: None,
            read_only: true, category: DataCategory::CurrentData, groups: None,
        }
    }

    fn stub_op_metadata() -> ::operation::OperationMetadata {
        ::operation::OperationMetadata {
            proximity_proof_required: false, synchronous_execution: true,
            exclusive_execution: false, supported_modes: None,
        }
    }

    // -----------------------------------------------------------------------
    // SOVD: empty collection builds and entity id is preserved
    // -----------------------------------------------------------------------
    #[test]
    fn test_sovd_build_empty() {
        let c = DiagnosticServicesCollectionBuilder::new(TestEntity { id: "e1".into() })
            .build().expect("empty build must succeed");
        assert_eq!(c.entity().entity_id(), "e1");
        assert!(c.read_resources().is_empty() && c.write_resources().is_empty()
            && c.data_resources().is_empty() && c.operations().is_empty());
    }

    // -----------------------------------------------------------------------
    // SOVD: registered services appear in metadata maps and per-item getters
    // -----------------------------------------------------------------------
    #[test]
    fn test_sovd_build_with_services() {
        let c = DiagnosticServicesCollectionBuilder::new(TestEntity { id: "e2".into() })
            .with_read_resource(StubReadResource, stub_metadata("r1"), ::common::JsonSchema::Null)
            .with_data_resource(StubDataResource, stub_metadata("d1"), ::common::JsonSchema::Null)
            .with_operation(StubOperation, "op1", stub_op_metadata())
            .build().expect("build must succeed");

        assert_eq!(c.read_resources().len(), 1);
        assert_eq!(c.data_resources().len(), 1);
        assert_eq!(c.operations().len(), 1);
        assert!(c.get_read_resource("r1").is_some());
        assert!(c.get_data_resource("d1").is_some());
        assert!(c.get_operation("op1").is_some());
    }

    // -----------------------------------------------------------------------
    // SOVD: duplicate ID across categories is rejected at build time
    // -----------------------------------------------------------------------
    #[test]
    fn test_sovd_duplicate_id_rejected() {
        let result = DiagnosticServicesCollectionBuilder::new(TestEntity { id: "e3".into() })
            .with_read_resource(StubReadResource, stub_metadata("dup"), ::common::JsonSchema::Null)
            .with_data_resource(StubDataResource, stub_metadata("dup"), ::common::JsonSchema::Null)
            .build();
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // SOVD: write resource appears in write_resources() and get_write_resource_mut()
    // -----------------------------------------------------------------------
    #[test]
    fn test_sovd_build_with_write_resource() {
        let mut c = DiagnosticServicesCollectionBuilder::new(TestEntity { id: "e4".into() })
            .with_write_resource(StubWritableDataResource, stub_metadata("w1"), ::common::JsonSchema::Null)
            .build().expect("build with write resource must succeed");

        assert_eq!(c.write_resources().len(), 1);
        assert!(c.write_resources().contains_key("w1"));
        let (meta, _schema) = &c.write_resources()["w1"];
        assert_eq!(meta.id, "w1");
        assert!(c.get_write_resource_mut("w1").is_some());
        assert!(c.get_write_resource_mut("missing").is_none());
    }

    // -----------------------------------------------------------------------
    // SOVD: duplicate ID between write resource and read resource is rejected
    // -----------------------------------------------------------------------
    #[test]
    fn test_sovd_write_resource_duplicate_id_rejected() {
        let result = DiagnosticServicesCollectionBuilder::new(TestEntity { id: "e5".into() })
            .with_read_resource(StubReadResource, stub_metadata("shared"), ::common::JsonSchema::Null)
            .with_write_resource(StubWritableDataResource, stub_metadata("shared"), ::common::JsonSchema::Null)
            .build();
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // UDS: empty collection builds
    // -----------------------------------------------------------------------
    #[test]
    fn test_uds_build_empty() {
        let c = UdsServicesCollectionBuilder::new().build().expect("empty UDS build must succeed");
        assert!(c.read_dids().is_empty() && c.write_dids().is_empty()
            && c.data_ids().is_empty() && c.routines().is_empty());
    }

    // -----------------------------------------------------------------------
    // UDS: registered services appear in IndexSets and per-item getters
    // -----------------------------------------------------------------------
    #[test]
    fn test_uds_build_with_services() {
        let c = UdsServicesCollectionBuilder::new()
            .with_read_did("F190", StubRdbi)
            .with_write_did("F191", StubWdbi)
            .with_routine("0301", StubRoutine)
            .build().expect("UDS build must succeed");

        assert!(c.read_dids().contains("F190") && c.write_dids().contains("F191")
            && c.routines().contains("0301"));
        assert!(c.get_read_did("F190").is_some() && c.get_routine("0301").is_some());
    }

    // -----------------------------------------------------------------------
    // UDS: combined read+write DID registered via with_data_id
    // -----------------------------------------------------------------------
    #[test]
    fn test_uds_build_with_data_id() {
        let c = UdsServicesCollectionBuilder::new()
            .with_data_id("F1A0", StubDataId)
            .build().expect("UDS build with data_id must succeed");

        assert_eq!(c.data_ids().len(), 1);
        assert!(c.data_ids().contains("F1A0"));
        assert!(c.get_data_id("F1A0").is_some());
        assert!(c.get_data_id("missing").is_none());
    }

    // -----------------------------------------------------------------------
    // UDS: duplicate ID across categories is rejected at build time
    // -----------------------------------------------------------------------
    #[test]
    fn test_uds_duplicate_id_rejected() {
        let result = UdsServicesCollectionBuilder::new()
            .with_read_did("F190", StubRdbi)
            .with_write_did("F190", StubWdbi)
            .build();
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // RegistrationHandle: deregister closure runs exactly once on drop
    // -----------------------------------------------------------------------
    #[test]
    fn test_registration_handle_deregisters_on_drop() {
        use std::sync::{Arc, atomic::{AtomicU32, Ordering}};
        let count = Arc::new(AtomicU32::new(0));
        let c = Arc::clone(&count);
        { let _h = RegistrationHandle::new(move || { c.fetch_add(1, Ordering::SeqCst); }); }
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
