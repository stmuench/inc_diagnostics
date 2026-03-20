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

use common::sovd::{DataError, GenericError};
use common::Result as DiagResult;
use common::{
    KeyValueAttributes, ReplyMessageEncoding, ReplyMessagePayload, RequestMessagePayload,
};

/*******************/
/* General Types   */
/*******************/
pub mod sovd {
    /// cf. ISO 17978-3:2025 Section 7.9.1, Table 70
    #[derive(Clone, Debug, PartialEq)]
    pub enum DataCategory {
        IdentData,
        CurrentData,
        StoredData,
        SysInfo,
        Custom(String),
    }

    impl std::fmt::Display for DataCategory {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                DataCategory::IdentData => write!(f, "identData"),
                DataCategory::CurrentData => write!(f, "currentData"),
                DataCategory::StoredData => write!(f, "storedData"),
                DataCategory::SysInfo => write!(f, "sysInfo"),
                DataCategory::Custom(name) => write!(f, "{}", name),
            }
        }
    }

    impl DataCategory {
        #[must_use]
        pub fn from_str(s: &str) -> Self {
            match s {
                "identData" => DataCategory::IdentData,
                "currentData" => DataCategory::CurrentData,
                "storedData" => DataCategory::StoredData,
                "sysInfo" => DataCategory::SysInfo,
                other => DataCategory::Custom(other.to_string()),
            }
        }
    }

    /// cf. ISO 17978-3:2025 Section 7.9.2.2, Table 73
    #[derive(Clone, Debug)]
    pub struct DataCategoryInfo {
        pub item: DataCategory,
        pub category_translation_id: Option<String>,
    }

    /*****************************/
    /* Data resource meta-data   */
    /*****************************/

    /// cf. ISO 17978-3:2025 Section 7.9.3.1, Table 81
    #[derive(Clone, Debug)]
    pub struct DataResourceMetadata {
        pub id: String,
        pub name: String,
        pub translation_id: Option<String>,
        pub category: DataCategory,
        pub groups: Option<Vec<String>>,
    }
}

/*******************************/
/* Read / Write value types    */
/*******************************/

/// cf. ISO 17978-3:2025 Section 7.14.6, Table 83
#[derive(Debug)]
pub struct ReadValueArgs {
    pub reply_encoding: ReplyMessageEncoding,
    pub additional_attrs: Option<KeyValueAttributes>,
}

impl ReadValueArgs {
    #[must_use]
    pub fn from(encoding: ReplyMessageEncoding) -> Self {
        Self {
            reply_encoding: encoding,
            additional_attrs: None,
        }
    }

    #[must_use]
    pub fn with_additional_attrs(mut self, additional_attrs: KeyValueAttributes) -> Self {
        self.additional_attrs = Some(additional_attrs);
        self
    }
}

/// cf. ISO 17978-3:2025 Section 7.9.3.2, Table 85
#[derive(Debug)]
pub struct ReadValueReply {
    pub id: Option<String>,
    pub data: ReplyMessagePayload,
    pub errors: Option<Vec<DataError>>,
}

/// cf. ISO 17978-3:2025 Section 7.9.5.1, Table 99
#[derive(Debug, Default)]
pub struct WriteValueArgs {
    pub signature: Option<String>,
    pub user_data: Option<RequestMessagePayload>,
    pub additional_attrs: Option<KeyValueAttributes>,
}

/*************************/
/* Data Resource API     */
/*************************/

/// Trait for a single data resource provider.
///
/// Implementations may optionally also provide write access to a specific data value
/// within an Entity, following the SOVD data resource model (ISO 17978-3 Section 7.9).
pub trait DataResource {
    /// Read the current value of this data resource.
    /// i.e. GET /{entity-path}/data/{data-id}
    fn read(&self, input: ReadValueArgs) -> DiagResult<ReadValueReply>;

    /// Write a new value to this data resource.
    /// i.e. PUT /{entity-path}/data/{data-id}
    /// The default implementation returns an error indicating that this data resource is read-only.
    fn write(&mut self, _input: WriteValueArgs) -> Result<(), DataError> {
        Err(DataError::from_error(GenericError::from_code(
            common::sovd::ErrorCode::PreconditionNotFulfilled,
            "This data resource cannot be written to since it is a read-only one!".to_string(),
        )))
    }
}
