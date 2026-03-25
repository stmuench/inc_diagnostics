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
