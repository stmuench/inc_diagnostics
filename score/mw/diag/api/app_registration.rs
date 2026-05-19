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
use common::{KeyValueAttributes, ReplyMessagePayload};
use futures::future::BoxFuture;

// Input payload for announcing an app endpoint to a diagnostics-facing registry.
#[derive(Clone, Debug, PartialEq)]
pub struct RegisterAppArgs {
    // Unique app identifier exposed through entity discovery.
    pub app_id: String,
    // Human-readable app name.
    pub app_name: String,
    // Hosting component identifier.
    pub hosted_on: String,
    // Transport endpoint used by a bridge or server to access app diagnostics.
    pub endpoint: String,
    // Optional app-specific metadata.
    pub additional_attrs: Option<KeyValueAttributes>,
}


// Result payload returned after app registration.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegisterAppReply {
    // Opaque registration handle, if the backend issues one.
    pub registration_id: Option<String>,
    // Optional lease window in milliseconds.
    pub lease_ms: Option<u64>,
}

// Input payload for removing an app endpoint from a diagnostics-facing registry.
#[derive(Clone, Debug, PartialEq)]
pub struct DeregisterAppArgs {
    // App identifier to remove.
    pub app_id: String,
    // Optional registration handle returned by [`RegisterAppReply`].
    pub registration_id: Option<String>,
}

/*
    Registry contract used by applications or bridges to register and deregister apps.
    Implementations can use REST, IPC, message buses, or in-process runtime calls.
*/
pub trait AppRegistrar {
    // Registers an app endpoint and returns optional lease information.
    fn register_app(&self, args: RegisterAppArgs) -> BoxFuture<'_, DiagResult<RegisterAppReply>>;

    // Removes a previously registered app endpoint.
    fn deregister_app(&self, args: DeregisterAppArgs) -> BoxFuture<'_, DiagResult<()>>;
}

// Optional lookup contract for bridges that need to resolve app endpoints.
pub trait AppRegistryQuery {
    // Resolves the latest endpoint for a registered app ID.
    fn resolve_endpoint(&self, app_id: &str) -> BoxFuture<'_, DiagResult<ReplyMessagePayload>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::ReplyMessagePayload;
    use common::sovd::{ErrorCode, GenericError};
    use futures::FutureExt;

    struct InMemoryRegistrar;

    impl AppRegistrar for InMemoryRegistrar {
        fn register_app(
            &self,
            args: RegisterAppArgs,
        ) -> BoxFuture<'_, DiagResult<RegisterAppReply>> {
            async move {
                if args.app_id.is_empty() {
                    return Err(common::Error::from_error(GenericError::from_code(
                        ErrorCode::IncompleteRequest,
                        "app_id must not be empty".to_string(),
                    )));
                }

                Ok(RegisterAppReply {
                    registration_id: Some("reg-1".to_string()),
                    lease_ms: Some(30_000),
                })
            }
            .boxed()
        }

        fn deregister_app(&self, _args: DeregisterAppArgs) -> BoxFuture<'_, DiagResult<()>> {
            async move { Ok(()) }.boxed()
        }
    }

    impl AppRegistryQuery for InMemoryRegistrar {
        fn resolve_endpoint(&self, _app_id: &str) -> BoxFuture<'_, DiagResult<ReplyMessagePayload>> {
            async move { Ok(ReplyMessagePayload::UTF8("http://127.0.0.1:8081/api".to_string())) }
                .boxed()
        }
    }

    #[tokio::test]
    async fn register_app_returns_registration_id() {
        let registrar = InMemoryRegistrar;
        let reply = registrar
            .register_app(RegisterAppArgs {
                app_id: "APP01".to_string(),
                app_name: "Diagnostics App".to_string(),
                hosted_on: "HPC".to_string(),
                endpoint: "http://127.0.0.1:8081/api".to_string(),
                additional_attrs: None,
            })
            .await
            .expect("registration should succeed");

        assert_eq!(reply.registration_id, Some("reg-1".to_string()));
        assert_eq!(reply.lease_ms, Some(30_000));
    }

    #[tokio::test]
    async fn register_app_rejects_empty_id() {
        let registrar = InMemoryRegistrar;
        let err = registrar
            .register_app(RegisterAppArgs {
                app_id: "".to_string(),
                app_name: "Diagnostics App".to_string(),
                hosted_on: "HPC".to_string(),
                endpoint: "http://127.0.0.1:8081/api".to_string(),
                additional_attrs: None,
            })
            .await
            .expect_err("registration should fail");

        match err.code {
            common::ErrorCode::SOVD(inner) => {
                assert_eq!(inner.sovd_error, ErrorCode::IncompleteRequest.to_string());
            }
            _ => panic!("expected SOVD error code"),
        }
    }
}
