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

use common::Result as DiagResult;
use common::*;
use data_resource::{DataResource, ReadValueArgs, ReadValueReply, WriteValueArgs};

/// UDS (Unified Diagnostic Services) data resource adapters.
///
/// Bridges UDS diagnostic services (ReadDataByIdentifier / WriteDataByIdentifier)
/// to the protocol-agnostic [`DataResource`](super::DataResource) trait.

/// A UDS data resource backed by either a read or write service.
pub enum DataResourceAdapter {
    /// Read-only data resource via ReadDataByIdentifier (0x22).
    RDBI(Box<dyn::uds::ReadDataByIdentifier + Send>),
    /// Write-only data resource via WriteDataByIdentifier (0x2E).
    WDBI(Box<dyn::uds::WriteDataByIdentifier + Send>),
}

impl DataResourceAdapter {
    #[must_use]
    pub fn from_rdbi(rdbi: impl ::uds::ReadDataByIdentifier + Send + 'static) -> Self {
        Self::RDBI { 0: Box::new(rdbi) }
    }

    #[must_use]
    pub fn from_wdbi(rdbi: impl ::uds::WriteDataByIdentifier + Send + 'static) -> Self {
        Self::WDBI { 0: Box::new(rdbi) }
    }
}

impl DataResource for DataResourceAdapter {
    fn read(&self, input: ReadValueArgs) -> DiagResult<ReadValueReply> {
        match self {
            Self::RDBI(rdbi) => match input.reply_encoding {
                ReplyMessageEncoding::Binary => {
                    let data = rdbi.read()?;
                    Ok(ReadValueReply {
                        id: None,
                        data: ReplyMessagePayload::from_byte_vector(data.to_vec()),
                        errors: None,
                    })
                }
                _ => Err(Error::from_error(sovd::GenericError::from_code(
                    sovd::ErrorCode::PreconditionNotFulfilled,
                    "UDS WriteDataByIdentifier only supports binary encoding for its reply!"
                        .to_string(),
                ))),
            },
            Self::WDBI(_) => Err(Error::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::PreconditionNotFulfilled,
                "UDS WriteDataByIdentifier does not permit any read operation!".to_string(),
            ))),
        }
    }

    fn write(&mut self, input: WriteValueArgs) -> std::result::Result<(), sovd::DataError> {
        match self {
            Self::RDBI(_) => Err(sovd::DataError::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::PreconditionNotFulfilled,
                "UDS ReadDataByIdentifier does not permit any write operation!".to_string(),
            ))),
            Self::WDBI(wdbi) => {
                if let Some(RequestMessagePayload::Binary(data)) = input.user_data {
                    wdbi.write(&data).map_err(|e| sovd::DataError {
                        path: String::new(),
                        error: match e.code {
                            ErrorCode::SOVD(err) => Some(err),
                            _ => Some(sovd::GenericError::from_code(
                                sovd::ErrorCode::ErrorResponse,
                                "Write operation failed".to_string(),
                            )),
                        },
                    })
                } else {
                    Err(sovd::DataError::from_error(sovd::GenericError::from_code(
                        sovd::ErrorCode::IncompleteRequest,
                        "Write operation requires binary payload data".to_string(),
                    )))
                }
            }
        }
    }
}

/*******************/
/* Unit Tests      */
/*******************/

#[cfg(test)]
mod tests {
    use super::*;
    use data_resource::DataResource;
    use {ByteSlice, ByteVector};

    // ── UDS RDBI / WDBI stub implementations ──────────────────────────

    struct RdbiForTest {
        data: ByteVector,
    }

    impl ::uds::ReadDataByIdentifier for RdbiForTest {
        fn read(&self) -> Result<ByteVector> {
            Ok(self.data.clone())
        }
    }

    struct FailingRdbi;

    impl ::uds::ReadDataByIdentifier for FailingRdbi {
        fn read(&self) -> Result<ByteVector> {
            Err(Error::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::NotResponding,
                "device not responding".to_string(),
            )))
        }
    }

    struct WdbiForTest {
        written: Option<ByteVector>,
    }

    impl ::uds::WriteDataByIdentifier for WdbiForTest {
        fn write(&mut self, input: ByteSlice) -> Result<()> {
            self.written = Some(input.to_vec());
            Ok(())
        }
    }

    struct FailingWdbi;

    impl ::uds::WriteDataByIdentifier for FailingWdbi {
        fn write(&mut self, _input: ByteSlice) -> Result<()> {
            Err(Error::from_error(sovd::GenericError::from_code(
                sovd::ErrorCode::ErrorResponse,
                "write failed".to_string(),
            )))
        }
    }

    // ── UDS DataResourceAdapter::read via RDBI ───────────────────────────────

    #[test]
    fn uds_rdbi_read_returns_binary_payload() {
        let resource = DataResourceAdapter::from_rdbi(RdbiForTest {
            data: vec![0xDE, 0xAD],
        });
        let result = resource
            .read(ReadValueArgs::from(ReplyMessageEncoding::Binary))
            .unwrap();
        assert_eq!(result.data, ReplyMessagePayload::Binary(vec![0xDE, 0xAD]));
        assert!(result.id.is_none());
        assert!(result.errors.is_none());
    }

    #[test]
    fn uds_rdbi_read_propagates_error() {
        let resource = DataResourceAdapter::from_rdbi(FailingRdbi {});
        let err = resource
            .read(ReadValueArgs::from(ReplyMessageEncoding::Binary))
            .unwrap_err();
        match err.code {
            ErrorCode::SOVD(ref e) => {
                assert_eq!(e.sovd_error, "not-responding");
            }
            _ => panic!("expected SOVD error code"),
        }
    }

    // ── UDS DataResourceAdapter::read via WDBI (should fail) ─────────────────

    #[test]
    fn uds_wdbi_read_returns_error() {
        let resource = DataResourceAdapter::from_wdbi(WdbiForTest { written: None });
        let err = resource
            .read(ReadValueArgs::from(ReplyMessageEncoding::Binary))
            .unwrap_err();
        match err.code {
            ErrorCode::SOVD(ref e) => {
                assert_eq!(e.sovd_error, "precondition-not-fulfilled");
            }
            _ => panic!("expected SOVD error code"),
        }
    }

    // ── UDS DataResourceAdapter::write via RDBI (should fail) ────────────────

    #[test]
    fn uds_rdbi_write_returns_error() {
        let mut resource = DataResourceAdapter::from_rdbi(RdbiForTest { data: vec![0x01] });
        let err = resource
            .write(WriteValueArgs {
                signature: None,
                user_data: Some(RequestMessagePayload::Binary(vec![0x01])),
                additional_attrs: None,
            })
            .unwrap_err();
        assert_eq!(
            err.error.as_ref().unwrap().sovd_error,
            "precondition-not-fulfilled"
        );
    }

    // ── UDS DataResourceAdapter::write via WDBI ──────────────────────────────

    #[test]
    fn uds_wdbi_write_succeeds_with_binary_data() {
        let mut resource = DataResourceAdapter::from_wdbi(WdbiForTest { written: None });
        let result = resource.write(WriteValueArgs {
            signature: None,
            user_data: Some(RequestMessagePayload::Binary(vec![0xCA, 0xFE])),
            additional_attrs: None,
        });
        assert!(result.is_ok());
    }

    #[test]
    fn uds_wdbi_write_fails_with_non_binary_payload() {
        let mut resource = DataResourceAdapter::from_wdbi(WdbiForTest { written: None });
        let err = resource
            .write(WriteValueArgs {
                signature: None,
                user_data: Some(RequestMessagePayload::UTF8("text".to_string())),
                additional_attrs: None,
            })
            .unwrap_err();
        assert_eq!(err.error.as_ref().unwrap().sovd_error, "incomplete-request");
    }

    #[test]
    fn uds_wdbi_write_fails_with_no_user_data() {
        let mut resource = DataResourceAdapter::from_wdbi(WdbiForTest { written: None });
        let err = resource
            .write(WriteValueArgs {
                signature: None,
                user_data: None,
                additional_attrs: None,
            })
            .unwrap_err();
        assert_eq!(err.error.as_ref().unwrap().sovd_error, "incomplete-request");
    }

    #[test]
    fn uds_wdbi_write_maps_underlying_error_to_data_error() {
        let mut resource = DataResourceAdapter::from_wdbi(FailingWdbi {});
        let err = resource
            .write(WriteValueArgs {
                signature: None,
                user_data: Some(RequestMessagePayload::Binary(vec![0x01])),
                additional_attrs: None,
            })
            .unwrap_err();
        assert_eq!(err.error.as_ref().unwrap().sovd_error, "error-response");
    }

    // ── UDS DataResourceAdapter::read with non-binary encoding ───────────────

    #[test]
    fn uds_rdbi_read_rejects_non_binary_encoding() {
        let resource = DataResourceAdapter::from_rdbi(RdbiForTest { data: vec![0x01] });
        let err = resource
            .read(ReadValueArgs::from(ReplyMessageEncoding::UTF8))
            .unwrap_err();
        match err.code {
            ErrorCode::SOVD(ref e) => {
                assert_eq!(e.sovd_error, "precondition-not-fulfilled");
            }
            _ => panic!("expected SOVD error code"),
        }
    }
}
