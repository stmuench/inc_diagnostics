# *******************************************************************************
# Copyright (c) 2026 Contributors to the Eclipse Foundation
#
# See the NOTICE file(s) distributed with this work for additional
# information regarding copyright ownership.
#
# This program and the accompanying materials are made available under the
# terms of the Apache License Version 2.0 which is available at
# https://www.apache.org/licenses/LICENSE-2.0
#
# SPDX-License-Identifier: Apache-2.0
# *******************************************************************************

load("@rules_rust//rust:defs.bzl", "rust_library", "rust_test")

rust_library(
    name = "lib_runtime",
    srcs = [
        "lib_runtime.rs",
        "opensovd_registrar.rs",
    ],
    deps = [
        "//score/mw/diag/api",
        "@crate_index//:futures",
        "@crate_index//:indexmap",
        "@crate_index//:serde_json",
        "@crate_index//:tokio",
    ],
)

rust_library(
    name = "diag_runtime",
    srcs = ["diag_runtime.rs"],
    deps = [
        ":lib_runtime",
    ],
)

rust_test(
    name = "runtime_registration_test",
    crate = ":lib_runtime",
    deps = [
        "@crate_index//:tokio",
    ],
)

alias(
    name = "runtime",
    actual = ":diag_runtime",
    visibility = [
        "//score:__subpackages__",
    ],
)
