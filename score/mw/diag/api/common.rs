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

pub type KeyValueAttributes = indexmap::IndexMap<String, String>;

pub mod sovd {
    use super::KeyValueAttributes;

    /// cf. ISO 17978-3:2025 Section 5.8.4, Table 18
    pub enum ErrorCode {
        ErrorResponse,
        IncompleteRequest,
        InsufficientAccessRights,
        InvalidResponseContent,
        InvalidSignature,
        LockBroken,
        NotResponding,
        PreconditionNotFulfilled,
        SovdServerFailure,
        SovdServerMisconfigured,
        UpdateAutomatedNotSupported,
        UpdateExecutionInProgress,
        UpdatePreparationInProgress,
        UpdateProcessInProgress,
        VendorSpecific,
    }

    impl std::fmt::Display for ErrorCode {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "{}",
                match self {
                    ErrorCode::ErrorResponse => "error-response",
                    ErrorCode::IncompleteRequest => "incomplete-request",
                    ErrorCode::InsufficientAccessRights => "insufficient-access-rights",
                    ErrorCode::InvalidResponseContent => "invalid-response-content",
                    ErrorCode::InvalidSignature => "invalid-signature",
                    ErrorCode::LockBroken => "lock-broken",
                    ErrorCode::NotResponding => "not-responding",
                    ErrorCode::PreconditionNotFulfilled => "precondition-not-fulfilled",
                    ErrorCode::SovdServerFailure => "sovd-server-failure",
                    ErrorCode::SovdServerMisconfigured => "sovd-server-misconfigured",
                    ErrorCode::UpdateAutomatedNotSupported => "update-automated-not-supported",
                    ErrorCode::UpdateExecutionInProgress => "update-execution-in-progress",
                    ErrorCode::UpdatePreparationInProgress => "update-preparation-in-progress",
                    ErrorCode::UpdateProcessInProgress => "update-process-in-progress",
                    ErrorCode::VendorSpecific => "vendor-specific",
                }
            )
        }
    }

    /// cf. ISO 17978-3:2025 Section 5.8.3, Table 16
    #[derive(Clone, Debug, PartialEq)]
    #[must_use]
    pub struct GenericError {
        pub sovd_error: String,
        pub message_text: String,
        pub vendor_error: Option<String>,
        pub translation_id: Option<String>,
        pub additional_attrs: Option<KeyValueAttributes>,
    }

    impl GenericError {
        pub fn from_code(code: ErrorCode, message: String) -> Self {
            Self {
                sovd_error: code.to_string(),
                message_text: message,
                vendor_error: None,
                translation_id: None,
                additional_attrs: None,
            }
        }

        pub fn from_vendor_error(error: String, message: String) -> Self {
            Self {
                sovd_error: ErrorCode::VendorSpecific.to_string(),
                message_text: message,
                vendor_error: Some(error),
                translation_id: None,
                additional_attrs: None,
            }
        }

        pub fn with_translation_id(mut self, translation_id: String) -> Self {
            self.translation_id = Some(translation_id);
            self
        }

        pub fn with_additional_attrs(mut self, attrs: KeyValueAttributes) -> Self {
            self.additional_attrs = Some(attrs);
            self
        }
    }

    pub type Error = GenericError;

    /// cf. ISO 17978-3:2025 Section 5.8.3, Table 17
    #[derive(Clone, Debug)]
    #[must_use]
    pub struct DataError {
        pub path: String,
        pub error: Option<GenericError>,
    }

    impl DataError {
        /// According to ISO 17978-3:2025 Section 5.8.3 Table 17, `path` shall contain
        /// a "JSON Pointer describing which element of the response is erroneous".
        pub fn new(path: String) -> Self {
            Self {
                path: path,
                error: None,
            }
        }

        /// Convenience factory method to create a `DataError` just from `GenericError`.
        pub fn from_error(error: GenericError) -> Self {
            Self {
                path: String::default(),
                error: Some(error),
            }
        }

        /// Instance method to enrich a newly created `DataError` with a `GenericError`.
        pub fn with_error(mut self, error: GenericError) -> Self {
            self.error = Some(error);
            self
        }
    }
}

pub mod uds {

    /// cf. ISO 14229-1:2020, Table A.1
    #[derive(Clone, Debug, PartialEq)]
    pub enum NegativeResponseCode {
        // TO BE ADDED
    }
}

pub type ByteSlice<'a> = &'a [u8];
pub type ByteVector = Vec<u8>;

pub type JsonValue = serde_json::Value;
pub type JsonSchema = serde_json::Value;

/// Representation of a request message payload for further processing.
#[derive(Clone, Debug, PartialEq)]
pub enum RequestMessagePayload {
    Binary(ByteVector),
    JSON(JsonValue),
    UTF8(String),
}

/// Indicates whether a JSON schema is required as part of a reply message.
#[derive(Clone, Debug, PartialEq)]
pub enum JsonSchemaRequired {
    Yes,
    No,
}

/// Expected encoding of a reply message, used to specify the desired response format.
#[derive(Clone, Debug, PartialEq)]
pub enum ReplyMessageEncoding {
    Binary,
    JSON(JsonSchemaRequired),
    UTF8,
}

/// Representation of a reply message payload to be sent back to clients.
#[derive(Clone, Debug, PartialEq)]
pub enum ReplyMessagePayload {
    Binary(ByteVector),
    JSON(JsonValue, Option<JsonSchema>),
    UTF8(String),
}

impl ReplyMessagePayload {
    #[must_use]
    pub fn from_byte_vector(payload: ByteVector) -> Self {
        Self::Binary(payload)
    }

    #[must_use]
    pub fn from_json(payload: JsonValue, schema: Option<JsonSchema>) -> Self {
        Self::JSON(payload, schema)
    }

    #[must_use]
    pub fn from_string(payload: String) -> Self {
        Self::UTF8(payload)
    }
}

/// Diagonstic reply type wrapping the reply message payload.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiagnosticReply {
    pub message_payload: Option<ReplyMessagePayload>,
    pub additional_attrs: Option<KeyValueAttributes>,
}

/// Diagnostic error code encompassing protocol-specific error variants.
#[derive(Clone, Debug, PartialEq)]
pub enum ErrorCode {
    SOVD(sovd::Error),
    UDS(uds::NegativeResponseCode),
}

/// Error type for any diagnostic action
#[derive(Clone, Debug, PartialEq)]
#[must_use]
pub struct Error {
    pub code: ErrorCode,
    pub payload: Option<ReplyMessagePayload>,
}

impl Error {
    pub fn from_error(error: sovd::Error) -> Self {
        Self {
            code: ErrorCode::SOVD(error),
            payload: None,
        }
    }

    pub fn from_nrc(nrc: uds::NegativeResponseCode) -> Self {
        Self {
            code: ErrorCode::UDS(nrc),
            payload: None,
        }
    }

    pub fn with_payload(mut self, payload: ReplyMessagePayload) -> Self {
        self.payload = Some(payload);
        self
    }

    pub fn mutex_poisoned() -> Self {
        Self::from_error(sovd::GenericError::from_code(
            sovd::ErrorCode::SovdServerFailure,
            "mutex acquisition failed unexpectedly".to_string(),
        ))
    }
}

/// Result type for any diagnostic action
pub type Result<T> = std::result::Result<T, Error>;

/*******************/
/* Unit Tests      */
/*******************/

#[cfg(test)]
mod tests {
    use super::*;

    // ── sovd::ErrorCode Display ────────────────────────────────────────

    #[test]
    fn error_code_display_error_response() {
        assert_eq!(sovd::ErrorCode::ErrorResponse.to_string(), "error-response");
    }

    #[test]
    fn error_code_display_incomplete_request() {
        assert_eq!(
            sovd::ErrorCode::IncompleteRequest.to_string(),
            "incomplete-request"
        );
    }

    #[test]
    fn error_code_display_insufficient_access_rights() {
        assert_eq!(
            sovd::ErrorCode::InsufficientAccessRights.to_string(),
            "insufficient-access-rights"
        );
    }

    #[test]
    fn error_code_display_invalid_response_content() {
        assert_eq!(
            sovd::ErrorCode::InvalidResponseContent.to_string(),
            "invalid-response-content"
        );
    }

    #[test]
    fn error_code_display_invalid_signature() {
        assert_eq!(
            sovd::ErrorCode::InvalidSignature.to_string(),
            "invalid-signature"
        );
    }

    #[test]
    fn error_code_display_lock_broken() {
        assert_eq!(sovd::ErrorCode::LockBroken.to_string(), "lock-broken");
    }

    #[test]
    fn error_code_display_not_responding() {
        assert_eq!(sovd::ErrorCode::NotResponding.to_string(), "not-responding");
    }

    #[test]
    fn error_code_display_precondition_not_fulfilled() {
        assert_eq!(
            sovd::ErrorCode::PreconditionNotFulfilled.to_string(),
            "precondition-not-fulfilled"
        );
    }

    #[test]
    fn error_code_display_sovd_server_failure() {
        assert_eq!(
            sovd::ErrorCode::SovdServerFailure.to_string(),
            "sovd-server-failure"
        );
    }

    #[test]
    fn error_code_display_sovd_server_misconfigured() {
        assert_eq!(
            sovd::ErrorCode::SovdServerMisconfigured.to_string(),
            "sovd-server-misconfigured"
        );
    }

    #[test]
    fn error_code_display_update_automated_not_supported() {
        assert_eq!(
            sovd::ErrorCode::UpdateAutomatedNotSupported.to_string(),
            "update-automated-not-supported"
        );
    }

    #[test]
    fn error_code_display_update_execution_in_progress() {
        assert_eq!(
            sovd::ErrorCode::UpdateExecutionInProgress.to_string(),
            "update-execution-in-progress"
        );
    }

    #[test]
    fn error_code_display_update_preparation_in_progress() {
        assert_eq!(
            sovd::ErrorCode::UpdatePreparationInProgress.to_string(),
            "update-preparation-in-progress"
        );
    }

    #[test]
    fn error_code_display_update_process_in_progress() {
        assert_eq!(
            sovd::ErrorCode::UpdateProcessInProgress.to_string(),
            "update-process-in-progress"
        );
    }

    #[test]
    fn error_code_display_vendor_specific() {
        assert_eq!(
            sovd::ErrorCode::VendorSpecific.to_string(),
            "vendor-specific"
        );
    }

    // ── sovd::GenericError constructors ────────────────────────────────

    #[test]
    fn generic_error_from_code() {
        let err = sovd::GenericError::from_code(
            sovd::ErrorCode::ErrorResponse,
            "test message".to_string(),
        );
        assert_eq!(err.sovd_error, "error-response");
        assert_eq!(err.message_text, "test message");
        assert!(err.vendor_error.is_none());
        assert!(err.translation_id.is_none());
        assert!(err.additional_attrs.is_none());
    }

    #[test]
    fn generic_error_with_translation_id() {
        let err =
            sovd::GenericError::from_code(sovd::ErrorCode::NotResponding, "timeout".to_string())
                .with_translation_id("tid_123".to_string());
        assert_eq!(err.translation_id.as_deref(), Some("tid_123"));
    }

    #[test]
    fn generic_error_from_code_with_translation() {
        let err = sovd::GenericError::from_code(
            sovd::ErrorCode::IncompleteRequest,
            "msg".to_string(),
        )
        .with_translation_id("trans_id".to_string());
        assert_eq!(err.sovd_error, "incomplete-request");
        assert_eq!(err.message_text, "msg");
        assert!(err.vendor_error.is_none());
        assert_eq!(err.translation_id.as_deref(), Some("trans_id"));
        assert!(err.additional_attrs.is_none());
    }

    #[test]
    fn generic_error_from_vendor_error() {
        let err =
            sovd::GenericError::from_vendor_error("custom-err".to_string(), "msg".to_string());
        assert_eq!(err.sovd_error, "vendor-specific");
        assert_eq!(err.message_text, "msg");
        assert_eq!(err.vendor_error.as_deref(), Some("custom-err"));
        assert!(err.translation_id.is_none());
        assert!(err.additional_attrs.is_none());
    }

    #[test]
    fn generic_error_from_vendor_error_with_translation() {
        let err =
            sovd::GenericError::from_vendor_error("custom-err".to_string(), "msg".to_string())
                .with_translation_id("tid".to_string());
        assert_eq!(err.sovd_error, "vendor-specific");
        assert_eq!(err.vendor_error.as_deref(), Some("custom-err"));
        assert_eq!(err.translation_id.as_deref(), Some("tid"));
    }

    // ── sovd::GenericError additional attrs ────────────────────────────

    #[test]
    fn generic_error_with_additional_attrs() {
        let mut attrs = KeyValueAttributes::new();
        attrs.insert("foo".to_string(), "bar".to_string());
        let err = sovd::GenericError::from_code(sovd::ErrorCode::ErrorResponse, "msg".to_string())
            .with_additional_attrs(attrs);
        let result_attrs = err.additional_attrs.as_ref().unwrap();
        assert_eq!(result_attrs.get("foo"), Some(&"bar".to_string()));
    }

    #[test]
    fn generic_error_with_additional_attrs_single_entry() {
        let mut attrs = KeyValueAttributes::new();
        attrs.insert("key1".to_string(), "val1".to_string());
        let err = sovd::GenericError::from_code(sovd::ErrorCode::ErrorResponse, "msg".to_string())
            .with_additional_attrs(attrs);
        let result_attrs = err.additional_attrs.as_ref().unwrap();
        assert_eq!(result_attrs.get("key1"), Some(&"val1".to_string()));
    }

    #[test]
    fn generic_error_with_additional_attrs_multiple_entries() {
        let mut attrs = KeyValueAttributes::new();
        attrs.insert("k1".to_string(), "v1".to_string());
        attrs.insert("k2".to_string(), "v2".to_string());
        let err = sovd::GenericError::from_code(sovd::ErrorCode::ErrorResponse, "msg".to_string())
            .with_additional_attrs(attrs);
        let result_attrs = err.additional_attrs.as_ref().unwrap();
        assert_eq!(result_attrs.len(), 2);
        assert_eq!(result_attrs.get("k1"), Some(&"v1".to_string()));
        assert_eq!(result_attrs.get("k2"), Some(&"v2".to_string()));
    }

    #[test]
    fn generic_error_with_additional_attrs_replaces() {
        let mut old_attrs = KeyValueAttributes::new();
        old_attrs.insert("old".to_string(), "val".to_string());
        let mut new_attrs = KeyValueAttributes::new();
        new_attrs.insert("new_key".to_string(), "new_val".to_string());
        let err = sovd::GenericError::from_code(sovd::ErrorCode::ErrorResponse, "msg".to_string())
            .with_additional_attrs(old_attrs)
            .with_additional_attrs(new_attrs);
        let attrs = err.additional_attrs.as_ref().unwrap();
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs.get("new_key"), Some(&"new_val".to_string()));
        assert!(attrs.get("old").is_none());
    }

    // ── sovd::GenericError clone ──────────────────────────────────────

    #[test]
    fn generic_error_clone() {
        let mut attrs = KeyValueAttributes::new();
        attrs.insert("k".to_string(), "v".to_string());
        let err = sovd::GenericError::from_code(sovd::ErrorCode::ErrorResponse, "msg".to_string())
            .with_additional_attrs(attrs);
        let cloned = err.clone();
        assert_eq!(cloned.sovd_error, err.sovd_error);
        assert_eq!(cloned.message_text, err.message_text);
        assert_eq!(cloned.additional_attrs, err.additional_attrs);
    }

    // ── sovd::DataError ───────────────────────────────────────────────

    #[test]
    fn data_error_new_with_path_and_error() {
        let generic_error =
            sovd::GenericError::from_code(sovd::ErrorCode::ErrorResponse, "msg".to_string());
        let data_err = sovd::DataError::new("/entity/data/1".to_string()).with_error(generic_error);
        assert_eq!(data_err.path, "/entity/data/1".to_string());
        assert!(data_err.error.is_some());
        assert_eq!(
            data_err.error.as_ref().unwrap().sovd_error,
            "error-response"
        );
    }

    #[test]
    fn data_error_with_lock_broken() {
        let generic_error =
            sovd::GenericError::from_code(sovd::ErrorCode::LockBroken, "locked".to_string());
        let data_err =
            sovd::DataError::new("/entity/data/123".to_string()).with_error(generic_error);
        assert_eq!(data_err.path, "/entity/data/123".to_string());
        assert!(data_err.error.is_some());
        assert_eq!(data_err.error.as_ref().unwrap().sovd_error, "lock-broken");
        assert_eq!(data_err.error.as_ref().unwrap().message_text, "locked");
    }

    #[test]
    fn data_error_clone() {
        let generic_error =
            sovd::GenericError::from_code(sovd::ErrorCode::LockBroken, "locked".to_string());
        let data_err =
            sovd::DataError::new("/entity/data/123".to_string()).with_error(generic_error);
        let cloned = data_err.clone();
        assert_eq!(cloned.path, data_err.path);
        assert_eq!(cloned.error, data_err.error);
    }

    #[test]
    fn data_error_with_error_builder() {
        let initial_error =
            sovd::GenericError::from_code(sovd::ErrorCode::ErrorResponse, "initial".to_string());
        let data_err = sovd::DataError::new("/path".to_string()).with_error(initial_error);
        let new_error = sovd::GenericError::from_code(
            sovd::ErrorCode::PreconditionNotFulfilled,
            "updated".to_string(),
        );
        let updated = data_err.with_error(new_error);
        assert_eq!(updated.path, "/path".to_string());
        assert_eq!(
            updated.error.as_ref().unwrap().sovd_error,
            "precondition-not-fulfilled"
        );
        assert_eq!(updated.error.as_ref().unwrap().message_text, "updated");
    }

    #[test]
    fn data_error_path_only() {
        let data_err: sovd::DataError = sovd::DataError {
            path: "/only/path".to_string(),
            error: None,
        };
        assert_eq!(data_err.path, "/only/path".to_string());
        assert!(data_err.error.is_none());
    }

    #[test]
    fn data_error_error_only() {
        let error = sovd::GenericError::from_code(
            sovd::ErrorCode::InvalidSignature,
            "signature invalid".to_string(),
        );
        let data_err = sovd::DataError::from_error(error.clone());
        assert_eq!(data_err.path, String::default());
        assert_eq!(
            data_err.error.as_ref().unwrap().sovd_error,
            "invalid-signature"
        );
    }

    // ── RequestMessagePayload ─────────────────────────────────────────

    #[test]
    fn request_message_payload_binary() {
        let payload = RequestMessagePayload::Binary(vec![0x01, 0x02, 0x03]);
        if let RequestMessagePayload::Binary(data) = &payload {
            assert_eq!(data, &[0x01, 0x02, 0x03]);
        } else {
            panic!("expected Binary variant");
        }
    }

    #[test]
    fn request_message_payload_json() {
        let json_val = serde_json::json!({"key": "value"});
        let payload = RequestMessagePayload::JSON(json_val.clone());
        assert_eq!(payload, RequestMessagePayload::JSON(json_val));
    }

    #[test]
    fn request_message_payload_utf8() {
        let payload = RequestMessagePayload::UTF8("hello".to_string());
        assert_eq!(payload, RequestMessagePayload::UTF8("hello".to_string()));
    }

    #[test]
    fn request_message_payload_clone() {
        let original = RequestMessagePayload::Binary(vec![42]);
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn request_message_payload_equality_different_variants() {
        let binary = RequestMessagePayload::Binary(vec![1]);
        let utf8 = RequestMessagePayload::UTF8("1".to_string());
        assert_ne!(binary, utf8);
    }

    // ── JsonSchemaRequired ────────────────────────────────────────────

    #[test]
    fn json_schema_required_clone_and_eq() {
        let yes = JsonSchemaRequired::Yes;
        let no = JsonSchemaRequired::No;
        assert_eq!(yes.clone(), JsonSchemaRequired::Yes);
        assert_eq!(no.clone(), JsonSchemaRequired::No);
        assert_ne!(yes, no);
    }

    // ── ReplyMessageEncoding ──────────────────────────────────────────

    #[test]
    fn reply_message_encoding_binary() {
        let enc = ReplyMessageEncoding::Binary;
        assert_eq!(enc, ReplyMessageEncoding::Binary);
    }

    #[test]
    fn reply_message_encoding_json_with_schema() {
        let enc = ReplyMessageEncoding::JSON(JsonSchemaRequired::Yes);
        assert_eq!(enc, ReplyMessageEncoding::JSON(JsonSchemaRequired::Yes));
    }

    #[test]
    fn reply_message_encoding_json_without_schema() {
        let enc = ReplyMessageEncoding::JSON(JsonSchemaRequired::No);
        assert_eq!(enc, ReplyMessageEncoding::JSON(JsonSchemaRequired::No));
    }

    #[test]
    fn reply_message_encoding_utf8() {
        let enc = ReplyMessageEncoding::UTF8;
        assert_eq!(enc, ReplyMessageEncoding::UTF8);
    }

    #[test]
    fn reply_message_encoding_clone() {
        let enc = ReplyMessageEncoding::JSON(JsonSchemaRequired::Yes);
        let cloned = enc.clone();
        assert_eq!(enc, cloned);
    }

    #[test]
    fn reply_message_encoding_inequality() {
        assert_ne!(ReplyMessageEncoding::Binary, ReplyMessageEncoding::UTF8);
        assert_ne!(
            ReplyMessageEncoding::JSON(JsonSchemaRequired::Yes),
            ReplyMessageEncoding::JSON(JsonSchemaRequired::No)
        );
    }

    // ── ReplyMessagePayload enum ──────────────────────────────────────

    #[test]
    fn reply_message_payload_binary() {
        let payload = ReplyMessagePayload::Binary(vec![1, 2]);
        if let ReplyMessagePayload::Binary(data) = &payload {
            assert_eq!(data, &[1, 2]);
        } else {
            panic!("expected Binary variant");
        }
    }

    #[test]
    fn reply_message_payload_json_variant() {
        let val = serde_json::json!("x");
        let schema = serde_json::json!({"type": "string"});
        let payload = ReplyMessagePayload::JSON(val.clone(), Some(schema.clone()));
        assert_eq!(payload, ReplyMessagePayload::JSON(val, Some(schema)));
    }

    #[test]
    fn reply_message_payload_clone() {
        let original = ReplyMessagePayload::UTF8("hello".to_string());
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn reply_message_payload_equality_different_variants() {
        let binary = ReplyMessagePayload::Binary(vec![1]);
        let utf8 = ReplyMessagePayload::UTF8("1".to_string());
        assert_ne!(binary, utf8);
    }

    // ── ReplyMessagePayload factory methods ───────────────────────────

    #[test]
    fn reply_message_payload_from_byte_vector() {
        let payload = ReplyMessagePayload::from_byte_vector(vec![0xAA, 0xBB]);
        assert_eq!(payload, ReplyMessagePayload::Binary(vec![0xAA, 0xBB]));
    }

    #[test]
    fn reply_message_payload_from_json() {
        let json_val = serde_json::json!({"a": 1});
        let payload = ReplyMessagePayload::from_json(json_val.clone(), None);
        assert_eq!(payload, ReplyMessagePayload::JSON(json_val, None));
    }

    #[test]
    fn reply_message_payload_from_json_with_schema() {
        let json_val = serde_json::json!({"a": 1});
        let schema = serde_json::json!({"type": "object"});
        let payload = ReplyMessagePayload::from_json(json_val.clone(), Some(schema.clone()));
        assert_eq!(payload, ReplyMessagePayload::JSON(json_val, Some(schema)));
    }

    #[test]
    fn reply_message_payload_from_string() {
        let payload = ReplyMessagePayload::from_string("text".to_string());
        assert_eq!(payload, ReplyMessagePayload::UTF8("text".to_string()));
    }

    // ── DiagnosticReply with ReplyMessagePayload::JSON ────────────────

    #[test]
    fn diagnostic_reply_with_json_payload() {
        let json_val = serde_json::json!({"result": true});
        let schema = serde_json::json!({"type": "object"});
        let reply = DiagnosticReply {
            message_payload: Some(ReplyMessagePayload::JSON(
                json_val.clone(),
                Some(schema.clone()),
            )),
            additional_attrs: None,
        };
        assert_eq!(
            reply.message_payload,
            Some(ReplyMessagePayload::JSON(json_val, Some(schema)))
        );
    }

    #[test]
    fn diagnostic_reply_empty() {
        let reply = DiagnosticReply {
            message_payload: None,
            additional_attrs: None,
        };
        assert!(reply.message_payload.is_none());
        assert!(reply.additional_attrs.is_none());
    }

    // ── DiagnosticReply ───────────────────────────────────────────────

    #[test]
    fn diagnostic_reply_with_payload() {
        let reply = DiagnosticReply {
            message_payload: Some(ReplyMessagePayload::UTF8("result".to_string())),
            additional_attrs: None,
        };
        assert_eq!(
            reply.message_payload,
            Some(ReplyMessagePayload::UTF8("result".to_string()))
        );
        assert!(reply.additional_attrs.is_none());
    }

    #[test]
    fn diagnostic_reply_with_attrs() {
        let mut attrs = KeyValueAttributes::new();
        attrs.insert("status".to_string(), "ok".to_string());
        let reply = DiagnosticReply {
            message_payload: None,
            additional_attrs: Some(attrs),
        };
        assert!(reply.message_payload.is_none());
        assert_eq!(
            reply.additional_attrs.as_ref().unwrap().get("status"),
            Some(&"ok".to_string())
        );
    }

    #[test]
    fn diagnostic_reply_clone() {
        let reply = DiagnosticReply {
            message_payload: Some(ReplyMessagePayload::Binary(vec![1, 2, 3])),
            additional_attrs: None,
        };
        let cloned = reply.clone();
        assert_eq!(reply, cloned);
    }

    #[test]
    fn diagnostic_reply_equality() {
        let reply_a = DiagnosticReply {
            message_payload: Some(ReplyMessagePayload::UTF8("a".to_string())),
            additional_attrs: None,
        };
        let reply_b = DiagnosticReply {
            message_payload: Some(ReplyMessagePayload::UTF8("b".to_string())),
            additional_attrs: None,
        };
        assert_ne!(reply_a, reply_b);
    }

    // ── Error / ErrorCode ─────────────────────────────────────────────

    #[test]
    fn error_from_sovd_error() {
        let sovd_err = sovd::GenericError::from_code(
            sovd::ErrorCode::SovdServerFailure,
            "server down".to_string(),
        );
        let err = Error::from_error(sovd_err);
        match err.code {
            ErrorCode::SOVD(inner) => {
                assert_eq!(inner.sovd_error, "sovd-server-failure");
            }
            _ => {
                panic!("expected SOVD error code");
            }
        };
        assert!(err.payload.is_none());
    }

    #[test]
    fn error_clone() {
        let err = Error::from_error(sovd::GenericError::from_code(
            sovd::ErrorCode::ErrorResponse,
            "msg".to_string(),
        ));
        let cloned = err.clone();
        if let (ErrorCode::SOVD(ref a), ErrorCode::SOVD(ref b)) = (&err.code, &cloned.code) {
            assert_eq!(a.sovd_error, b.sovd_error);
            assert_eq!(a.message_text, b.message_text);
        } else {
            panic!("expected SOVD variants");
        }
    }

    // ── Result type alias ─────────────────────────────────────────────

    #[test]
    fn result_ok_variant() {
        let result: Result<i32> = Ok(42);
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn result_err_variant() {
        let result: Result<i32> = Err(Error::from_error(sovd::GenericError::from_code(
            sovd::ErrorCode::ErrorResponse,
            "fail".to_string(),
        )));
        assert!(result.is_err());
    }

    // ── Error with payload ────────────────────────────────────────────

    #[test]
    fn error_from_sovd_error_with_payload() {
        let sovd_err =
            sovd::GenericError::from_code(sovd::ErrorCode::ErrorResponse, "msg".to_string());
        let mut err = Error::from_error(sovd_err);
        assert!(err.payload.is_none());
        err.payload = Some(ReplyMessagePayload::UTF8("detail".to_string()));
        assert_eq!(
            err.payload,
            Some(ReplyMessagePayload::UTF8("detail".to_string()))
        );
    }

    #[test]
    fn error_with_payload_builder() {
        let sovd_err =
            sovd::GenericError::from_code(sovd::ErrorCode::ErrorResponse, "msg".to_string());
        let err =
            Error::from_error(sovd_err).with_payload(ReplyMessagePayload::Binary(vec![0xCA, 0xFE]));
        assert_eq!(
            err.payload,
            Some(ReplyMessagePayload::Binary(vec![0xCA, 0xFE]))
        );
    }

    #[test]
    fn error_mutex_error() {
        let err = Error::mutex_poisoned();
        match &err.code {
            ErrorCode::SOVD(inner) => {
                assert_eq!(inner.sovd_error, "sovd-server-failure");
                assert_eq!(inner.message_text, "mutex acquisition failed unexpectedly");
            }
            _ => {
                panic!("expected SOVD error code");
            }
        }
        assert!(err.payload.is_none());
    }

    // ── DiagnosticReply Default ───────────────────────────────────────

    #[test]
    fn diagnostic_reply_default() {
        let reply = DiagnosticReply::default();
        assert!(reply.message_payload.is_none());
        assert!(reply.additional_attrs.is_none());
    }
}
