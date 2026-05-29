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

use indexmap::IndexMap;

use ::common::{Error, Result};

use ::data_resource::{DataResource, ReadOnlyDataResource, WritableDataResource};
use ::operation::Operation;
use ::uds::{ReadDataByIdentifier, WriteDataByIdentifier, RoutineControl};

// ============================================================================
// SOVD Builder Pattern
// ============================================================================

/// Trait that diagnostic entities must implement to be registered with SOVD services
pub trait DiagnosticEntity {
    /// Returns a unique identifier for this diagnostic entity
    fn entity_id(&self) -> &str;
}

/// SOVD Service Collection Builder with fluent API for service registration
pub struct DiagnosticServicesCollectionBuilder<'entity> {
    entity: Box<dyn DiagnosticEntity + Send + Sync + 'entity>,
    read_resources: IndexMap<String, Box<dyn ReadOnlyDataResource + Send + Sync + 'static>>,
    write_resources: IndexMap<String, Box<dyn WritableDataResource + Send + 'static>>,
    data_resources: IndexMap<String, Box<dyn DataResource + Send + Sync + 'static>>,
    operations: IndexMap<String, Box<dyn Operation + Send + Sync + 'static>>,
}

impl<'entity> DiagnosticServicesCollectionBuilder<'entity> {
    /// Creates a new SOVD services builder
    pub fn new<T: DiagnosticEntity + Send + Sync + 'entity>(entity: T) -> Self {
        Self {
            entity: Box::new(entity),
            read_resources: IndexMap::new(),
            write_resources: IndexMap::new(),
            data_resources: IndexMap::new(),
            operations: IndexMap::new(),
        }
    }

    /// Registers a read-only data resource
    pub fn with_read_resource<T: ReadOnlyDataResource + Send + Sync + 'static>(
        mut self,
        id: impl Into<String>,
        resource: T,
    ) -> Self {
        self.read_resources.insert(id.into(), Box::new(resource));
        self
    }

    /// Registers a write-only data resource
    pub fn with_write_resource<T: WritableDataResource + Send + 'static>(
        mut self,
        id: impl Into<String>,
        resource: T,
    ) -> Self {
        self.write_resources.insert(id.into(), Box::new(resource));
        self
    }

    /// Registers a read-write data resource
    pub fn with_data_resource<T: DataResource + Send + Sync + 'static>(
        mut self,
        id: impl Into<String>,
        resource: T,
    ) -> Self {
        self.data_resources.insert(id.into(), Box::new(resource));
        self
    }

    /// Registers an operation
    pub fn with_operation<T: Operation + Send + Sync + 'static>(
        mut self,
        id: impl Into<String>,
        operation: T,
    ) -> Self {
        self.operations.insert(id.into(), Box::new(operation));
        self
    }

    /// Builds and returns the final DiagnosticServicesCollection
    pub fn build(self) -> Result<DiagnosticServicesCollection<'entity>> {
        self.validate()?;
        Ok(DiagnosticServicesCollection {
            entity: self.entity,
            read_resources: self.read_resources,
            write_resources: self.write_resources,
            data_resources: self.data_resources,
            operations: self.operations,
        })
    }

    fn validate(&self) -> Result<()> {
        let all_ids: Vec<_> = self.read_resources.keys()
            .chain(self.write_resources.keys())
            .chain(self.data_resources.keys())
            .chain(self.operations.keys())
            .collect();
        let unique: std::collections::HashSet<_> = all_ids.iter().collect();
        if unique.len() != all_ids.len() {
            return Err(Error::from_error(
                ::common::sovd::GenericError::from_code(
                    ::common::sovd::ErrorCode::SovdServerMisconfigured,
                    "Duplicate service ID detected".to_string(),
                ),
            ));
        }
        Ok(())
    }
}

/// SOVD Services Collection from builder pattern
///
/// Contains registered services for an entity with access methods
pub struct DiagnosticServicesCollection<'entity> {
    entity: Box<dyn DiagnosticEntity + Send + Sync + 'entity>,
    read_resources: IndexMap<String, Box<dyn ReadOnlyDataResource + Send + Sync + 'static>>,
    write_resources: IndexMap<String, Box<dyn WritableDataResource + Send + 'static>>,
    data_resources: IndexMap<String, Box<dyn DataResource + Send + Sync + 'static>>,
    operations: IndexMap<String, Box<dyn Operation + Send + Sync + 'static>>,
}

impl<'entity> DiagnosticServicesCollection<'entity> {
    /// Returns reference to the entity
    pub fn entity(&self) -> &(dyn DiagnosticEntity + Send + Sync) {
        &*self.entity
    }

    /// Returns all registered read resources
    pub fn get_read_resources(&self) -> &IndexMap<String, Box<dyn ReadOnlyDataResource + Send + Sync + 'static>> {
        &self.read_resources
    }

    /// Returns all registered write resources (mutable)
    pub fn get_write_resources_mut(&mut self) -> &mut IndexMap<String, Box<dyn WritableDataResource + Send + 'static>> {
        &mut self.write_resources
    }

    /// Returns all registered data resources
    pub fn get_data_resources(&self) -> &IndexMap<String, Box<dyn DataResource + Send + Sync + 'static>> {
        &self.data_resources
    }

    /// Returns all registered operations
    pub fn get_operations(&self) -> &IndexMap<String, Box<dyn Operation + Send + Sync + 'static>> {
        &self.operations
    }
}

// ============================================================================
// Service Registration Binding Layer
// ============================================================================

/// Binding layer between user-built service collections and the runtime.
/// User code obtains a [`ServiceRegistrar`] from the runtime and passes collections to it.
pub trait ServiceRegistrar {
    /// Registers a SOVD service collection for the given entity with the runtime.
    fn register_sovd_services<'entity>(
        &self,
        collection: DiagnosticServicesCollection<'entity>,
    ) -> Result<()>;

    /// Registers a UDS service collection (DIDs and routines) with the runtime.
    fn register_uds_services(&self, collection: UdsServicesCollection) -> Result<()>;
}

// ============================================================================
// UDS Builder Pattern
// ============================================================================

/// Entity-agnostic builder for UDS diagnostic services (DIDs and routines)
#[derive(Default)]
pub struct UdsServicesCollectionBuilder {
    read_dids: IndexMap<String, Box<dyn ReadDataByIdentifier + Send + 'static>>,
    write_dids: IndexMap<String, Box<dyn WriteDataByIdentifier + Send + 'static>>,
    routines: IndexMap<String, Box<dyn RoutineControl + Send + 'static>>,
}

impl UdsServicesCollectionBuilder {
    /// Creates a new UDS services builder
    pub fn new() -> Self {
        Self {
            read_dids: IndexMap::new(),
            write_dids: IndexMap::new(),
            routines: IndexMap::new(),
        }
    }

    /// Registers a read Data Identifier (DID)
    pub fn with_read_did<T: ReadDataByIdentifier + Send + 'static>(
        mut self,
        id: impl Into<String>,
        service: T,
    ) -> Self {
        self.read_dids.insert(id.into(), Box::new(service));
        self
    }

    /// Registers a write Data Identifier (DID)
    pub fn with_write_did<T: WriteDataByIdentifier + Send + 'static>(
        mut self,
        id: impl Into<String>,
        service: T,
    ) -> Self {
        self.write_dids.insert(id.into(), Box::new(service));
        self
    }

    /// Registers a routine control handler
    pub fn with_routine<T: RoutineControl + Send + 'static>(
        mut self,
        id: impl Into<String>,
        routine: T,
    ) -> Self {
        self.routines.insert(id.into(), Box::new(routine));
        self
    }

    /// Builds and returns the final UdsServicesCollection
    pub fn build(self) -> Result<UdsServicesCollection> {
        self.validate()?;
        Ok(UdsServicesCollection {
            read_dids: self.read_dids,
            write_dids: self.write_dids,
            routines: self.routines,
        })
    }

    fn validate(&self) -> Result<()> {
        let all_ids: Vec<_> = self.read_dids.keys()
            .chain(self.write_dids.keys())
            .chain(self.routines.keys())
            .collect();
        let unique: std::collections::HashSet<_> = all_ids.iter().collect();
        if unique.len() != all_ids.len() {
            return Err(Error::from_error(
                ::common::sovd::GenericError::from_code(
                    ::common::sovd::ErrorCode::SovdServerMisconfigured,
                    "Duplicate service ID detected".to_string(),
                ),
            ));
        }
        Ok(())
    }
}

/// UDS Services Collection from builder pattern
pub struct UdsServicesCollection {
    read_dids: IndexMap<String, Box<dyn ReadDataByIdentifier + Send + 'static>>,
    write_dids: IndexMap<String, Box<dyn WriteDataByIdentifier + Send + 'static>>,
    routines: IndexMap<String, Box<dyn RoutineControl + Send + 'static>>,
}

impl UdsServicesCollection {
    /// Returns all registered read DIDs
    pub fn get_read_dids(&self) -> &IndexMap<String, Box<dyn ReadDataByIdentifier + Send + 'static>> {
        &self.read_dids
    }

    /// Returns all registered write DIDs (mutable)
    pub fn get_write_dids_mut(&mut self) -> &mut IndexMap<String, Box<dyn WriteDataByIdentifier + Send + 'static>> {
        &mut self.write_dids
    }

    /// Returns all registered routines
    pub fn get_routines(&self) -> &IndexMap<String, Box<dyn RoutineControl + Send + 'static>> {
        &self.routines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test entity for SOVD tests
    struct TestEntity {
        id: String,
    }

    impl DiagnosticEntity for TestEntity {
        fn entity_id(&self) -> &str {
            &self.id
        }
    }

    // Basic SOVD Builder Tests
    #[test]
    fn test_sovd_collection_build() {
        let entity = TestEntity {
            id: "vehicle_001".to_string(),
        };
        let collection = DiagnosticServicesCollectionBuilder::new(entity)
            .build()
            .expect("Failed to build collection");

        assert_eq!(collection.entity().entity_id(), "vehicle_001");
    }

    // Basic UDS Builder Tests
    #[test]
    fn test_uds_builder_builds_empty_collection() {
        let collection = UdsServicesCollectionBuilder::new()
            .build()
            .expect("Failed to build UDS collection");
        assert!(collection.get_read_dids().is_empty());
        assert!(collection.get_write_dids_mut().is_empty());
        assert!(collection.get_routines().is_empty());
    }

    #[test]
    fn test_uds_builder_default_equals_new() {
        let _collection = UdsServicesCollectionBuilder::default()
            .build()
            .expect("Failed to build UDS collection with default");
    }
}
