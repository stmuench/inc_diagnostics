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

use diag_api::sovd::app_registration::{
    AppRegistrar, DeregisterAppArgs, RegisterAppArgs, RegisterAppReply,
};
use diag_api::sovd;
use diag_api::Error as DiagError;
use diag_api::Result as DiagResult;

use futures::future::BoxFuture;
use futures::FutureExt;

use serde_json::{json, Value};

use std::io::{Read, Write};
use std::net::TcpStream;

#[derive(Clone, Debug)]
pub struct OpenSovdRegistrar {
    base_url: HttpUrl,
}

impl OpenSovdRegistrar {
    pub fn new(base_url: impl AsRef<str>) -> DiagResult<Self> {
        let parsed = HttpUrl::parse(base_url.as_ref())?;

        Ok(Self { base_url: parsed })
    }

    fn register_path(&self) -> String {
        self.base_url.join_path("register")
    }

    fn deregister_path(&self, app_id: &str) -> String {
        self.base_url.join_path(&format!("register/{app_id}"))
    }
}

impl AppRegistrar for OpenSovdRegistrar {
    fn register_app(&self, args: RegisterAppArgs) -> BoxFuture<'_, DiagResult<RegisterAppReply>> {
        async move {
            let endpoint = HttpUrl::parse(&args.endpoint).map_err(|_| {
                invalid_request(format!("invalid app endpoint '{}'", args.endpoint))
            })?;

            let response = send_http_request(
                &self.base_url,
                "POST",
                &self.register_path(),
                Some(
                    json!({
                        "app_id": args.app_id,
                        "app_name": args.app_name,
                        "hosted_on": args.hosted_on,
                        "port": endpoint.port,
                    })
                    .to_string(),
                ),
            )?;

            if !(200..300).contains(&response.status_code) {
                return Err(backend_error_response(format!(
                    "register request failed with status {}",
                    response.status_code
                )));
            }

            Ok(parse_register_reply(&response.body))
        }
        .boxed()
    }

    fn deregister_app(&self, args: DeregisterAppArgs) -> BoxFuture<'_, DiagResult<()>> {
        async move {
            let response = send_http_request(
                &self.base_url,
                "DELETE",
                &self.deregister_path(&args.app_id),
                None,
            )?;

            if !(200..300).contains(&response.status_code) {
                return Err(backend_error_response(format!(
                    "deregister request failed with status {}",
                    response.status_code
                )));
            }

            Ok(())
        }
        .boxed()
    }
}

fn parse_register_reply(response_body: &str) -> RegisterAppReply {
    if response_body.trim().is_empty() {
        return RegisterAppReply::default();
    }

    let Ok(value) = serde_json::from_str::<Value>(response_body) else {
        return RegisterAppReply::default();
    };

    RegisterAppReply {
        registration_id: value
            .get("registration_id")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        lease_ms: value.get("lease_ms").and_then(Value::as_u64),
    }
}

fn invalid_request(message: String) -> DiagError {
    DiagError::from_error(sovd::GenericError::from_code(
        sovd::ErrorCode::IncompleteRequest,
        message,
    ))
}

fn backend_error_response(message: String) -> DiagError {
    DiagError::from_error(sovd::GenericError::from_code(
        sovd::ErrorCode::ErrorResponse,
        message,
    ))
}

fn backend_failure(message: String) -> DiagError {
    DiagError::from_error(sovd::GenericError::from_code(
        sovd::ErrorCode::SovdServerFailure,
        message,
    ))
}

#[derive(Clone, Debug)]
struct HttpUrl {
    host: String,
    port: u16,
    path: String,
}

impl HttpUrl {
    fn parse(raw_url: &str) -> DiagResult<Self> {
        let trimmed = raw_url.trim();
        let rest = trimmed
            .strip_prefix("http://")
            .ok_or_else(|| invalid_request("only http URLs are supported".to_string()))?;

        let (authority, path_suffix) = match rest.split_once('/') {
            Some((authority, path_suffix)) => (authority, format!("/{}", path_suffix)),
            None => (rest, "/".to_string()),
        };

        let (host, port) = authority.rsplit_once(':').ok_or_else(|| {
            invalid_request("URL must include host and port".to_string())
        })?;

        if host.is_empty() {
            return Err(invalid_request("URL host must not be empty".to_string()));
        }

        let port = port
            .parse::<u16>()
            .map_err(|_| invalid_request("URL port must be a valid u16".to_string()))?;

        Ok(Self {
            host: host.to_string(),
            port,
            path: normalize_path(&path_suffix),
        })
    }

    fn join_path(&self, suffix: &str) -> String {
        let suffix = suffix.trim_start_matches('/');
        if self.path == "/" {
            format!("/{suffix}")
        } else {
            format!("{}/{}", self.path.trim_end_matches('/'), suffix)
        }
    }
}

struct HttpResponse {
    status_code: u16,
    body: String,
}

fn normalize_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        "/".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.trim_end_matches('/').to_string()
    } else {
        format!("/{}", trimmed.trim_end_matches('/'))
    }
}

fn send_http_request(
    base_url: &HttpUrl,
    method: &str,
    path: &str,
    body: Option<String>,
) -> DiagResult<HttpResponse> {
    let mut stream = TcpStream::connect((base_url.host.as_str(), base_url.port))
        .map_err(|err| backend_failure(format!("request connection failed: {err}")))?;

    let body = body.unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        base_url.host,
        base_url.port,
        body.len(),
        body
    );

    stream
        .write_all(request.as_bytes())
        .map_err(|err| backend_failure(format!("failed to send request: {err}")))?;

    let mut raw_response = String::new();
    stream
        .read_to_string(&mut raw_response)
        .map_err(|err| backend_failure(format!("failed to read response: {err}")))?;

    let (head, body) = raw_response.split_once("\r\n\r\n").ok_or_else(|| {
        backend_failure("response did not contain an HTTP header/body separator".to_string())
    })?;
    let status_line = head.lines().next().ok_or_else(|| {
        backend_failure("response did not contain an HTTP status line".to_string())
    })?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| backend_failure("response status line missing status code".to_string()))?
        .parse::<u16>()
        .map_err(|_| backend_failure("response status code was not numeric".to_string()))?;

    Ok(HttpResponse {
        status_code,
        body: body.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[derive(Debug)]
    struct CapturedRequest {
        method: String,
        path: String,
        body: String,
    }

    #[tokio::test]
    async fn new_rejects_https_scheme() {
        let err = OpenSovdRegistrar::new("https://127.0.0.1:7790/api")
            .expect_err("https should be rejected");

        match err.code {
            diag_api::ErrorCode::SOVD(inner) => {
                assert_eq!(inner.sovd_error, sovd::ErrorCode::IncompleteRequest.to_string());
            }
            _ => panic!("expected SOVD error code"),
        }
    }

    #[tokio::test]
    async fn register_app_rejects_invalid_endpoint() {
        let registrar = OpenSovdRegistrar::new("http://127.0.0.1:7790/api")
            .expect("base URL should be valid");

        let err = registrar
            .register_app(RegisterAppArgs {
                app_id: "APP01".to_string(),
                app_name: "Diagnostics App".to_string(),
                hosted_on: "HPC".to_string(),
                endpoint: "not-a-url".to_string(),
                additional_attrs: None,
            })
            .await
            .expect_err("invalid endpoint should fail");

        match err.code {
            diag_api::ErrorCode::SOVD(inner) => {
                assert_eq!(inner.sovd_error, sovd::ErrorCode::IncompleteRequest.to_string());
            }
            _ => panic!("expected SOVD error code"),
        }
    }

    #[tokio::test]
    async fn register_app_posts_to_register_endpoint() {
        let (base_url, request_handle) = spawn_test_server(
            "HTTP/1.1 200 OK\r\nContent-Length: 44\r\nContent-Type: application/json\r\n\r\n{\"registration_id\":\"reg-7\",\"lease_ms\":5000}",
        );
        let registrar = OpenSovdRegistrar::new(base_url).expect("base URL should be valid");

        let reply = registrar
            .register_app(RegisterAppArgs {
                app_id: "APP01".to_string(),
                app_name: "Diagnostics App".to_string(),
                hosted_on: "HPC".to_string(),
                endpoint: "http://127.0.0.1:8081/api".to_string(),
                additional_attrs: None,
            })
            .await
            .expect("register should succeed");

        let request = request_handle.join().expect("server thread should finish");
        let body: Value = serde_json::from_str(&request.body).expect("body should be valid JSON");

        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/register");
        assert_eq!(body["app_id"], "APP01");
        assert_eq!(body["app_name"], "Diagnostics App");
        assert_eq!(body["hosted_on"], "HPC");
        assert_eq!(body["port"], 8081);
        assert_eq!(reply.registration_id, Some("reg-7".to_string()));
        assert_eq!(reply.lease_ms, Some(5000));
    }

    #[tokio::test]
    async fn deregister_app_sends_delete_request() {
        let (base_url, request_handle) = spawn_test_server(
            "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n",
        );
        let registrar = OpenSovdRegistrar::new(base_url).expect("base URL should be valid");

        registrar
            .deregister_app(DeregisterAppArgs {
                app_id: "APP02".to_string(),
                registration_id: None,
            })
            .await
            .expect("deregister should succeed");

        let request = request_handle.join().expect("server thread should finish");
        assert_eq!(request.method, "DELETE");
        assert_eq!(request.path, "/api/register/APP02");
        assert!(request.body.is_empty());
    }

    fn spawn_test_server(response: &'static str) -> (String, thread::JoinHandle<CapturedRequest>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener.local_addr().expect("listener should have local addr");

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("server should accept one connection");
            let mut buffer = Vec::new();
            let mut temp = [0_u8; 4096];
            let mut header_end = None;

            while header_end.is_none() {
                let read = stream.read(&mut temp).expect("request should be readable");
                buffer.extend_from_slice(&temp[..read]);
                header_end = buffer
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|idx| idx + 4);
            }

            let header_end = header_end.expect("header terminator should exist");
            let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    if name.eq_ignore_ascii_case("Content-Length") {
                        value.trim().parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);

            while buffer.len() < header_end + content_length {
                let read = stream.read(&mut temp).expect("request body should be readable");
                buffer.extend_from_slice(&temp[..read]);
            }

            stream
                .write_all(response.as_bytes())
                .expect("response should be writable");

            let request_line = headers.lines().next().expect("request line should exist");
            let mut parts = request_line.split_whitespace();
            let method = parts.next().expect("method should exist").to_string();
            let path = parts.next().expect("path should exist").to_string();
            let body = String::from_utf8(buffer[header_end..header_end + content_length].to_vec())
                .expect("body should be utf8");

            CapturedRequest { method, path, body }
        });

        (format!("http://{}/api", address), handle)
    }
}
