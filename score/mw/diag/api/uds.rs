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
use common::{ByteSlice, ByteVector};
use std::future::Future;
use std::pin::Pin;

/// UDS ReadDataByIdentifier service (cf. ISO 14229-1:2020, Service 0x22).
pub trait ReadDataByIdentifier {
    /// Read raw bytes for the data identifier.
    fn read(&self) -> DiagResult<ByteVector>;
}

/// UDS WriteDataByIdentifier service (cf. ISO 14229-1:2020, Service 0x2E).
pub trait WriteDataByIdentifier {
    /// Write raw bytes for the data identifier.
    fn write(&mut self, input: ByteSlice) -> DiagResult<()>;
}

/// UDS RoutineControl service (cf. ISO 14229-1:2020, Service 0x31).
/// NOTE: request routine results (sub-function 0x03) will get handled
///       implicitly by the diag runtime via `ExecutionEvent::ReportStatus`.
pub trait RoutineControl {
    /// Start a routine (sub-function 0x01).
    fn start(&mut self, input: Option<ByteSlice>) -> DiagResult<StartRoutine>;

    /// Stop a routine (sub-function 0x02).
    fn stop(&mut self, input: Option<ByteSlice>) -> DiagResult<Option<ByteVector>>;

    /// Optionally provide the current completion percentage
    fn completion_percentage(&self) -> Option<u8> {
        None
    }
}

/// Returned by `RoutineControl::start` and contains a future (which optionally produces
/// a `ByteVector` as execution result), along with an optional `ByteVector` which
/// shall get used as reply to the `RoutineControl::start` request.
#[must_use]
pub struct StartRoutine {
    pub future: Pin<Box<dyn Future<Output = DiagResult<Option<ByteVector>>> + Send>>,
    pub reply: Option<ByteVector>,
}

impl StartRoutine {
    pub fn from_closure<Func>(func: Func, reply: Option<ByteVector>) -> DiagResult<Self>
    where
        Func: FnOnce() -> DiagResult<Option<ByteVector>> + Send + 'static,
    {
        Ok(Self {
            future: Box::pin(async move { func() }),
            reply,
        })
    }

    pub fn from_future<Fut>(future: Fut, reply: Option<ByteVector>) -> DiagResult<Self>
    where
        Fut: Future<Output = DiagResult<Option<ByteVector>>> + Send + 'static,
    {
        Ok(Self {
            future: Box::pin(future),
            reply,
        })
    }

    pub fn from_nrc(nrc: ::common::uds::NegativeResponseCode) -> DiagResult<Self> {
        Err(::common::Error::from_nrc(nrc))
    }
}

/***************************************/
/* UDS Binary Serialization Traits     */
/***************************************/

/// Serialize a typed value into a raw UDS binary payload.
pub trait UdsSerialize {
    fn serialize(&self) -> DiagResult<ByteVector>;
}

/// Deserialize a typed value from bytes in a UDS service request.
pub trait UdsDeserialize: Sized {
    fn deserialize(data: ByteSlice) -> DiagResult<Self>;
}

/***************************************/
/* Type-Safe Service Handler Traits    */
/***************************************/

/// Used with [`SerializedWriteDataByIdentifier`]; the adapter decodes the raw
/// bytes via [`UdsDeserialize`] before forwarding the typed value here.
pub trait WriteHandler<T> {
    fn handle_write(&mut self, data: T) -> DiagResult<()>;
}

/// Used with [`SerializedRoutineControl`]; the adapter handles all
/// [`UdsSerialize`]/[`UdsDeserialize`] calls automatically.
pub trait RoutineHandler<T> {
    /// Start the routine; the returned value is serialized and sent as the immediate reply.
    fn start(&mut self, params: Option<T>) -> DiagResult<Option<T>>;

    /// Stop the routine; the returned value is serialized and sent as the stop reply.
    fn stop(&mut self, params: Option<T>) -> DiagResult<Option<T>>;

    /// Return the current routine results, surfaced via `ReportStatus`.
    /// Returns `Ok(None)` by default (sub-function not supported).
    fn results(&self) -> DiagResult<Option<T>> {
        Ok(None)
    }

    /// Optionally report the current completion percentage.
    fn completion_percentage(&self) -> Option<u8> {
        None
    }
}

/***************************************/
/* Serialization Helper Utilities      */
/***************************************/

/// Stateless encode/decode utilities for UDS payloads.
pub struct SerializationHelper;

impl SerializationHelper {
    /// Encode `payload` into raw bytes via [`UdsSerialize::serialize`].
    ///
    /// Any internal serialization or formatting failure is normalized to a UDS
    /// `FailurePreventsExecutionOfRequestedAction` code before propagating.
    pub fn serialize_response<T: UdsSerialize>(payload: &T) -> DiagResult<ByteVector> {
        payload.serialize().map_err(|_| {
            ::common::Error::from_nrc(
                ::common::uds::NegativeResponseCode::FailurePreventsExecutionOfRequestedAction,
            )
        })
    }

    /// Deserializes raw `data` into a `T` type via [`UdsDeserialize`], 
    /// then invokes the `handler` closure with the typed value.
    ///
    /// Any internal parsing or format failure is normalized to a UDS wire-level
    /// `IncorrectMessageLengthOrInvalidFormat` code before propagating.
    pub fn deserialize_request<T, F>(data: ByteSlice, handler: F) -> DiagResult<()>
    where
        T: UdsDeserialize,
        F: FnOnce(T) -> DiagResult<()>,
    {
        let parsed_request = T::deserialize(data).map_err(|_| {
            ::common::Error::from_nrc(
                ::common::uds::NegativeResponseCode::IncorrectMessageLengthOrInvalidFormat,
            )
        })?;
        handler(parsed_request)
    }
}

/*********************************************/
/* Serialized Service Wrapper Structs        */
/*********************************************/

/// Wraps a `T: UdsSerialize` value and exposes it as [`ReadDataByIdentifier`].
/// `read()` calls `T::serialize()` automatically.
pub struct SerializedReadDataByIdentifier<T: UdsSerialize + Send + Sync + 'static> {
    data: T,
}

impl<T: UdsSerialize + Send + Sync + 'static> SerializedReadDataByIdentifier<T> {
    pub fn new(data: T) -> Self {
        Self { data }
    }
}

impl<T: UdsSerialize + Send + Sync + 'static> ReadDataByIdentifier for SerializedReadDataByIdentifier<T> {
    fn read(&self) -> DiagResult<ByteVector> {
        self.data.serialize().map_err(|_| {
            ::common::Error::from_nrc(
                ::common::uds::NegativeResponseCode::FailurePreventsExecutionOfRequestedAction,
            )
        })
    }
}

/// Wraps a [`WriteHandler<T>`] and exposes it as [`WriteDataByIdentifier`].
/// Decodes raw bytes via `T::deserialize` before forwarding to the handler.
pub struct SerializedWriteDataByIdentifier<T, H>
where
    T: UdsDeserialize + Send + Sync + 'static,
    H: WriteHandler<T> + Send + Sync + 'static,
{
    handler: H,
    _phantom: std::marker::PhantomData<T>,
}

impl<T, H> SerializedWriteDataByIdentifier<T, H>
where
    T: UdsDeserialize + Send + Sync + 'static,
    H: WriteHandler<T> + Send + Sync + 'static,
{
    pub fn new(handler: H) -> Self {
        Self { 
            handler,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T, H> WriteDataByIdentifier for SerializedWriteDataByIdentifier<T, H>
where
    T: UdsDeserialize + Send + Sync + 'static,
    H: WriteHandler<T> + Send + Sync + 'static,
{
    fn write(&mut self, input: ByteSlice) -> DiagResult<()> {
        let value = T::deserialize(input).map_err(|_| {
            ::common::Error::from_nrc(
                ::common::uds::NegativeResponseCode::IncorrectMessageLengthOrInvalidFormat,
            )
        })?;
        self.handler.handle_write(value)
    }
}

/// Wraps a [`RoutineHandler<T>`] and exposes it as [`RoutineControl`].
/// Decodes inputs via [`UdsDeserialize`] and encodes outputs via [`UdsSerialize`]
/// automatically. `start` runs the handler synchronously and places the
/// serialized result in `StartRoutine::reply`; the future resolves `Ok(None)`.
pub struct SerializedRoutineControl<T, H>
where
    T: UdsSerialize + UdsDeserialize + Send + Sync + 'static,
    H: RoutineHandler<T> + Send + Sync + 'static,
{
    handler: H,
    _phantom: std::marker::PhantomData<T>,
}

impl<T, H> SerializedRoutineControl<T, H>
where
    T: UdsSerialize + UdsDeserialize + Send + Sync + 'static,
    H: RoutineHandler<T> + Send + Sync + 'static,
{
    #[must_use]
    pub fn new(handler: H) -> Self {
        Self { 
            handler,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T, H> RoutineControl for SerializedRoutineControl<T, H>
where
    T: UdsSerialize + UdsDeserialize + Send + Sync + 'static,
    H: RoutineHandler<T> + Send + Sync + 'static,
{
    fn start(&mut self, input: Option<ByteSlice>) -> DiagResult<StartRoutine> {
        let params = input
            .map(|bytes| {
                T::deserialize(bytes).map_err(|_| {
                    ::common::Error::from_nrc(
                        ::common::uds::NegativeResponseCode::IncorrectMessageLengthOrInvalidFormat,
                    )
                })
            })
            .transpose()?;

        let reply_bytes = self
            .handler
            .start(params)?
            .map(|v| {
                v.serialize().map_err(|_| {
                    ::common::Error::from_nrc(
                        ::common::uds::NegativeResponseCode::FailurePreventsExecutionOfRequestedAction,
                    )
                })
            })
            .transpose()?;
        StartRoutine::from_future(std::future::ready(Ok(None)), reply_bytes)
    }

    fn stop(&mut self, input: Option<ByteSlice>) -> DiagResult<Option<ByteVector>> {
        let params = input
            .map(|bytes| {
                T::deserialize(bytes).map_err(|_| {
                    ::common::Error::from_nrc(
                        ::common::uds::NegativeResponseCode::IncorrectMessageLengthOrInvalidFormat,
                    )
                })
            })
            .transpose()?;

        self.handler
            .stop(params)?
            .map(|v| {
                v.serialize().map_err(|_| {
                    ::common::Error::from_nrc(
                        ::common::uds::NegativeResponseCode::FailurePreventsExecutionOfRequestedAction,
                    )
                })
            })
            .transpose()
    }

    fn completion_percentage(&self) -> Option<u8> {
        self.handler.completion_percentage()
    }
}

/*******************/
/* Unit Tests      */
/*******************/

#[cfg(test)]
mod tests {
    use super::*;

    // Two-byte big-endian u16 — shared across all tests.
    #[derive(Debug, Clone, PartialEq)]
    struct Speed { value: u16 }

    impl UdsSerialize for Speed {
        fn serialize(&self) -> DiagResult<ByteVector> {
            Ok(self.value.to_be_bytes().to_vec())
        }
    }

    impl UdsDeserialize for Speed {
        fn deserialize(data: ByteSlice) -> DiagResult<Self> {
            if data.len() < 2 {
                return Err(::common::Error::from_nrc(
                    ::common::uds::NegativeResponseCode::IncorrectMessageLengthOrInvalidFormat,
                ));
            }
            Ok(Self { value: u16::from_be_bytes([data[0], data[1]]) })
        }
    }

    fn expect_nrc(err: ::common::Error, expected: ::common::uds::NegativeResponseCode) {
        match err.code {
            ::common::ErrorCode::UDS(nrc) => assert_eq!(nrc, expected),
            _ => panic!("expected UDS NRC"),
        }
    }

    // Verify the UdsSerialize/UdsDeserialize impls round-trip correctly and
    // that a too-short input returns IncorrectMessageLengthOrInvalidFormat.
    #[test]
    fn serialize_deserialize_round_trip() {
        assert_eq!(Speed { value: 0x0078 }.serialize().unwrap(), vec![0x00, 0x78]);
        assert_eq!(Speed::deserialize(&[0x00, 0x78]).unwrap().value, 120);
        expect_nrc(
            Speed::deserialize(&[0x01]).unwrap_err(),
            ::common::uds::NegativeResponseCode::IncorrectMessageLengthOrInvalidFormat,
        );
    }

    // SerializedReadDataByIdentifier: read() must return the serialized value.
    #[test]
    fn serialized_rdbi_read() {
        let rdbi = SerializedReadDataByIdentifier::new(Speed { value: 0xABCD });
        assert_eq!(rdbi.read().unwrap(), vec![0xAB, 0xCD]);
    }

    // SerializedWriteDataByIdentifier: bytes are decoded before reaching the handler;
    // a too-short payload must be rejected with the correct NRC.
    #[test]
    fn serialized_wdbi_write() {
        struct Capture { last: Option<Speed> }
        impl WriteHandler<Speed> for Capture {
            fn handle_write(&mut self, data: Speed) -> DiagResult<()> {
                self.last = Some(data);
                Ok(())
            }
        }

        let mut wdbi = SerializedWriteDataByIdentifier::<Speed, _>::new(Capture { last: None });
        wdbi.write(&[0x00, 0x64]).unwrap();
        assert_eq!(wdbi.handler.last.unwrap().value, 100);

        let mut wdbi = SerializedWriteDataByIdentifier::<Speed, _>::new(Capture { last: None });
        expect_nrc(
            wdbi.write(&[0x01]).unwrap_err(),
            ::common::uds::NegativeResponseCode::IncorrectMessageLengthOrInvalidFormat,
        );
    }

    // SerializedRoutineControl: start echoes the decoded value as a serialized reply
    // and the long-running future resolves Ok(None); stop echoes likewise.
    // A too-short input is rejected before reaching the handler.
    #[tokio::test]
    async fn serialized_routine_start_stop() {
        struct Echo;
        impl RoutineHandler<Speed> for Echo {
            fn start(&mut self, p: Option<Speed>) -> DiagResult<Option<Speed>> { Ok(p) }
            fn stop(&mut self,  p: Option<Speed>) -> DiagResult<Option<Speed>> { Ok(p) }
        }

        let mut r = SerializedRoutineControl::<Speed, _>::new(Echo);

        let start = r.start(Some(&[0x00, 0x96])).unwrap();
        assert_eq!(start.reply, Some(vec![0x00, 0x96]));
        assert!(start.future.await.unwrap().is_none());

        assert_eq!(r.stop(Some(&[0x00, 0x64])).unwrap(), Some(vec![0x00, 0x64]));

        match r.start(Some(&[0x01])) {
            Err(err) => expect_nrc(err, ::common::uds::NegativeResponseCode::IncorrectMessageLengthOrInvalidFormat),
            Ok(_) => panic!("Expected parsing format error block layout, but got Ok statement"),
        }
    }
}
