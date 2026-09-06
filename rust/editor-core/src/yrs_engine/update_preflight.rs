use serde_json::json;

use crate::boundary::ResourceLimits;

use super::{raw_storage_work_limit, YrsEngineError, YrsEngineResult};

pub(super) fn preflight_update_v1(encoded: &[u8], limits: &ResourceLimits) -> YrsEngineResult<()> {
    if encoded.len() > limits.max_encoded_state_bytes {
        return Err(YrsEngineError::limit(
            "DOCUMENT_LIMIT_EXCEEDED",
            limits.max_encoded_state_bytes,
            encoded.len(),
        )
        .with_details(json!({ "field": "encodedState", "phase": "updatePreflight" })));
    }
    let mut reader = PreflightReader::new(encoded, limits);
    reader.read_update()?;
    if reader.remaining() != 0 {
        return Err(reader.decode_error("trailingBytes"));
    }
    Ok(())
}

struct PreflightReader<'a> {
    bytes: &'a [u8],
    offset: usize,
    limits: &'a ResourceLimits,
    work: usize,
    payload: usize,
    json_string_payload: usize,
}

impl<'a> PreflightReader<'a> {
    fn new(bytes: &'a [u8], limits: &'a ResourceLimits) -> Self {
        Self {
            bytes,
            offset: 0,
            limits,
            work: 0,
            payload: 0,
            json_string_payload: 0,
        }
    }

    fn read_update(&mut self) -> YrsEngineResult<()> {
        let clients = self.read_var_u32()? as usize;
        self.require_declared_count(clients, 3)?;
        self.charge_work(clients)?;
        for _ in 0..clients {
            let blocks = self.read_var_u32()? as usize;
            self.require_declared_count(blocks, 1)?;
            self.read_client_id()?;
            let mut clock = self.read_var_u32()?;
            for _ in 0..blocks {
                self.charge_work(1)?;
                let len = self.read_block()?;
                clock = clock
                    .checked_add(len)
                    .ok_or_else(|| self.decode_error("clockOverflow"))?;
            }
        }
        self.read_delete_set()
    }

    fn read_block(&mut self) -> YrsEngineResult<u32> {
        let info = self.read_u8()?;
        if info == 10 || info == 0 {
            return self.read_var_u32();
        }
        if info & 0x80 != 0 {
            self.read_id()?;
        }
        if info & 0x40 != 0 {
            self.read_id()?;
        }
        if info & 0xc0 == 0 {
            match self.read_var_u32()? {
                1 => {
                    self.read_string()?;
                }
                0 => self.read_id()?,
                _ => return Err(self.decode_error("parentInfo")),
            }
            if info & 0x20 != 0 {
                self.read_string()?;
            }
        }
        self.read_content(info & 0x0f)
    }

    fn read_content(&mut self, tag: u8) -> YrsEngineResult<u32> {
        match tag {
            1 => self.read_var_u32(),
            2 => {
                let len = self.read_var_u32()?;
                let count = len
                    .checked_add(1)
                    .ok_or_else(|| self.decode_error("declaredLength"))?;
                self.require_declared_count(count as usize, 1)?;
                self.charge_work(count as usize)?;
                for _ in 0..count {
                    self.read_string()?;
                }
                Ok(count)
            }
            3 => {
                self.read_buffer()?;
                Ok(1)
            }
            4 => {
                let text = self.read_string()?;
                u32::try_from(text.encode_utf16().count())
                    .map_err(|_| self.decode_error("stringLength"))
            }
            5 => {
                self.read_json()?;
                Ok(1)
            }
            6 => {
                let key_bytes = self.read_string()?.len();
                self.charge_json_string_payload(key_bytes)?;
                self.read_json()?;
                Ok(1)
            }
            7 => {
                let type_ref = self.read_u8()?;
                match type_ref {
                    0 | 1 | 2 | 4 | 5 | 6 | 9 | 15 => {}
                    3 => {
                        self.read_string()?;
                    }
                    _ => return Err(self.decode_error("typeRef")),
                }
                Ok(1)
            }
            8 => {
                let count = self.read_var_u32()?;
                self.require_declared_count(count as usize, 1)?;
                for _ in 0..count {
                    self.read_any(1)?;
                }
                Ok(count)
            }
            9 => {
                self.read_string()?;
                self.read_any(1)?;
                Ok(1)
            }
            11 => {
                let flags = self.skip_varint(5)? as u32;
                self.read_id()?;
                if flags & 1 == 0 {
                    self.read_id()?;
                }
                Ok(1)
            }
            _ => Err(self.decode_error("contentTag")),
        }
    }

    fn read_any(&mut self, depth: usize) -> YrsEngineResult<()> {
        if depth > self.limits.max_document_depth {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                self.limits.max_document_depth,
                depth,
            )
            .with_details(json!({
                "field": "encodedState",
                "phase": "updatePreflight",
                "dimension": "anyDepth"
            })));
        }
        self.charge_work(1)?;
        match self.read_u8()? {
            127 | 126 | 121 | 120 => Ok(()),
            125 => {
                self.skip_varint(10)?;
                Ok(())
            }
            124 => self.skip(4),
            123 | 122 => self.skip(8),
            119 => {
                self.read_string()?;
                Ok(())
            }
            118 => {
                let count = self.read_var_u32()? as usize;
                self.require_declared_count(count, 2)?;
                for _ in 0..count {
                    self.read_string()?;
                    self.read_any(depth + 1)?;
                }
                Ok(())
            }
            117 => {
                let count = self.read_var_u32()? as usize;
                self.require_declared_count(count, 1)?;
                for _ in 0..count {
                    self.read_any(depth + 1)?;
                }
                Ok(())
            }
            116 => {
                self.read_buffer()?;
                Ok(())
            }
            _ => Err(self.decode_error("anyTag")),
        }
    }

    fn read_delete_set(&mut self) -> YrsEngineResult<()> {
        let clients = self.read_var_u32()? as usize;
        self.require_declared_count(clients, 2)?;
        self.charge_work(clients)?;
        for _ in 0..clients {
            self.read_client_id()?;
            let ranges = self.read_var_u32()? as usize;
            self.require_declared_count(ranges, 2)?;
            for _ in 0..ranges {
                self.charge_work(1)?;
                let clock = self.read_var_u32()?;
                let len = self.read_var_u32()?;
                clock
                    .checked_add(len)
                    .ok_or_else(|| self.decode_error("deleteClockOverflow"))?;
            }
        }
        Ok(())
    }

    fn read_id(&mut self) -> YrsEngineResult<()> {
        self.read_client_id()?;
        self.read_var_u32()?;
        Ok(())
    }

    fn read_string(&mut self) -> YrsEngineResult<&'a str> {
        let bytes = self.read_buffer()?;
        std::str::from_utf8(bytes).map_err(|_| self.decode_error("invalidUtf8"))
    }

    fn read_json(&mut self) -> YrsEngineResult<()> {
        let source = self.read_string()?;
        let scanner = JsonPreflight::new(source, self.limits, self.work, self.json_string_payload);
        let (work, string_payload) = scanner.scan()?;
        self.work = work;
        self.json_string_payload = string_payload;
        Ok(())
    }

    fn charge_json_string_payload(&mut self, amount: usize) -> YrsEngineResult<()> {
        self.json_string_payload = self
            .json_string_payload
            .checked_add(amount)
            .ok_or_else(|| self.decode_error("jsonStringPayloadOverflow"))?;
        if self.json_string_payload > self.limits.max_input_bytes {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                self.limits.max_input_bytes,
                self.json_string_payload,
            )
            .with_details(json!({
                "field": "encodedState",
                "phase": "updatePreflight",
                "dimension": "jsonStringBytes"
            })));
        }
        Ok(())
    }

    fn read_buffer(&mut self) -> YrsEngineResult<&'a [u8]> {
        let len = self.read_var_u32()? as usize;
        if len > self.remaining() {
            return Err(self.declared_length_error());
        }
        self.payload = self
            .payload
            .checked_add(len)
            .ok_or_else(|| self.decode_error("payloadOverflow"))?;
        if self.payload > self.limits.max_encoded_state_bytes {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                self.limits.max_encoded_state_bytes,
                self.payload,
            )
            .with_details(json!({
                "field": "encodedState",
                "phase": "updatePreflight",
                "dimension": "payloadBytes"
            })));
        }
        self.take(len)
    }

    fn read_var_u32(&mut self) -> YrsEngineResult<u32> {
        let value = self.skip_varint(5)?;
        u32::try_from(value).map_err(|_| self.decode_error("invalidVarint"))
    }

    fn read_client_id(&mut self) -> YrsEngineResult<u64> {
        let value = self.skip_varint(8)?;
        if value >= 1_u64 << 53 {
            return Err(self.decode_error("invalidVarint"));
        }
        Ok(value)
    }

    fn skip_varint(&mut self, max_bytes: usize) -> YrsEngineResult<u64> {
        let mut value = 0_u64;
        for index in 0..max_bytes {
            let byte = self.read_u8()?;
            let payload = u64::from(byte & 0x7f);
            let shift = index * 7;
            value = value
                .checked_add(
                    payload
                        .checked_shl(u32::try_from(shift).unwrap_or(u32::MAX))
                        .ok_or_else(|| self.decode_error("invalidVarint"))?,
                )
                .ok_or_else(|| self.decode_error("invalidVarint"))?;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(self.decode_error("invalidVarint"))
    }

    fn require_declared_count(&self, count: usize, minimum_bytes: usize) -> YrsEngineResult<()> {
        if count > self.remaining() / minimum_bytes {
            return Err(self.declared_length_error());
        }
        let limit = raw_storage_work_limit(self.limits);
        if count > limit {
            // Decoded update collections describe raw CRDT storage, which can
            // legitimately be more fragmented than the canonical document.
            // Bound that amplification independently from canonical nodes.
            return Err(
                YrsEngineError::limit("DOCUMENT_LIMIT_EXCEEDED", limit, count).with_details(
                    json!({
                        "field": "encodedState",
                        "phase": "updatePreflight",
                        "dimension": "collectionItems"
                    }),
                ),
            );
        }
        Ok(())
    }

    fn charge_work(&mut self, amount: usize) -> YrsEngineResult<()> {
        self.work = self
            .work
            .checked_add(amount)
            .ok_or_else(|| self.decode_error("workOverflow"))?;
        let limit = raw_storage_work_limit(self.limits);
        if self.work > limit {
            return Err(
                YrsEngineError::limit("DOCUMENT_LIMIT_EXCEEDED", limit, self.work).with_details(
                    json!({
                        "field": "encodedState",
                        "phase": "updatePreflight",
                        "dimension": "work"
                    }),
                ),
            );
        }
        Ok(())
    }

    fn read_u8(&mut self) -> YrsEngineResult<u8> {
        Ok(self.take(1)?[0])
    }

    fn skip(&mut self, len: usize) -> YrsEngineResult<()> {
        self.take(len).map(|_| ())
    }

    fn take(&mut self, len: usize) -> YrsEngineResult<&'a [u8]> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| self.decode_error("truncated"))?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| self.decode_error("truncated"))?;
        self.offset = end;
        Ok(slice)
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn declared_length_error(&self) -> YrsEngineError {
        YrsEngineError::new(
            "COLLABORATION_DECODE_FAILED",
            "update-v1 declared length exceeds the remaining encoded state",
        )
        .with_details(json!({
            "field": "encodedState",
            "phase": "updatePreflight",
            "reason": "declaredLength"
        }))
    }

    fn decode_error(&self, reason: &'static str) -> YrsEngineError {
        YrsEngineError::new(
            "COLLABORATION_DECODE_FAILED",
            "encoded snapshot state failed update-v1 structural preflight",
        )
        .with_details(json!({
            "field": "encodedState",
            "phase": "updatePreflight",
            "reason": reason
        }))
    }
}

struct JsonPreflight<'a> {
    bytes: &'a [u8],
    offset: usize,
    limits: &'a ResourceLimits,
    work: usize,
    string_payload: usize,
}

impl<'a> JsonPreflight<'a> {
    fn new(
        source: &'a str,
        limits: &'a ResourceLimits,
        work: usize,
        string_payload: usize,
    ) -> Self {
        Self {
            bytes: source.as_bytes(),
            offset: 0,
            limits,
            work,
            string_payload,
        }
    }

    fn scan(mut self) -> YrsEngineResult<(usize, usize)> {
        self.skip_whitespace();
        self.read_value(1)?;
        self.skip_whitespace();
        if self.offset != self.bytes.len() {
            return Err(self.invalid_json());
        }
        Ok((self.work, self.string_payload))
    }

    fn read_value(&mut self, depth: usize) -> YrsEngineResult<()> {
        if depth > self.limits.max_document_depth {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                self.limits.max_document_depth,
                depth,
            )
            .with_details(json!({
                "field": "encodedState",
                "phase": "updatePreflight",
                "dimension": "jsonDepth"
            })));
        }
        self.charge_work(1)?;
        match self.peek() {
            Some(b'n') => self.read_literal(b"null"),
            Some(b't') => self.read_literal(b"true"),
            Some(b'f') => self.read_literal(b"false"),
            Some(b'"') => self.read_string(),
            Some(b'[') => self.read_array(depth),
            Some(b'{') => self.read_object(depth),
            Some(b'-' | b'0'..=b'9') => self.read_number(),
            _ => Err(self.invalid_json()),
        }
    }

    fn read_array(&mut self, depth: usize) -> YrsEngineResult<()> {
        self.offset += 1;
        self.skip_whitespace();
        if self.consume(b']') {
            return Ok(());
        }
        loop {
            self.read_value(depth + 1)?;
            self.skip_whitespace();
            if self.consume(b']') {
                return Ok(());
            }
            if !self.consume(b',') {
                return Err(self.invalid_json());
            }
            self.skip_whitespace();
        }
    }

    fn read_object(&mut self, depth: usize) -> YrsEngineResult<()> {
        self.offset += 1;
        self.skip_whitespace();
        if self.consume(b'}') {
            return Ok(());
        }
        loop {
            if self.peek() != Some(b'"') {
                return Err(self.invalid_json());
            }
            self.read_string()?;
            self.skip_whitespace();
            if !self.consume(b':') {
                return Err(self.invalid_json());
            }
            self.skip_whitespace();
            self.read_value(depth + 1)?;
            self.skip_whitespace();
            if self.consume(b'}') {
                return Ok(());
            }
            if !self.consume(b',') {
                return Err(self.invalid_json());
            }
            self.skip_whitespace();
        }
    }

    fn read_string(&mut self) -> YrsEngineResult<()> {
        if !self.consume(b'"') {
            return Err(self.invalid_json());
        }
        loop {
            let byte = self.next().ok_or_else(|| self.invalid_json())?;
            match byte {
                b'"' => return Ok(()),
                0x00..=0x1f => return Err(self.invalid_json()),
                b'\\' => self.read_escape()?,
                _ => self.charge_string_payload(1)?,
            }
        }
    }

    fn read_escape(&mut self) -> YrsEngineResult<()> {
        match self.next().ok_or_else(|| self.invalid_json())? {
            b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => self.charge_string_payload(1),
            b'u' => {
                let first = self.read_hex_quad()?;
                if (0xd800..=0xdbff).contains(&first) {
                    if self.next() != Some(b'\\') || self.next() != Some(b'u') {
                        return Err(self.invalid_json());
                    }
                    let second = self.read_hex_quad()?;
                    if !(0xdc00..=0xdfff).contains(&second) {
                        return Err(self.invalid_json());
                    }
                    self.charge_string_payload(4)
                } else if (0xdc00..=0xdfff).contains(&first) {
                    Err(self.invalid_json())
                } else {
                    let character =
                        char::from_u32(u32::from(first)).ok_or_else(|| self.invalid_json())?;
                    self.charge_string_payload(character.len_utf8())
                }
            }
            _ => Err(self.invalid_json()),
        }
    }

    fn read_hex_quad(&mut self) -> YrsEngineResult<u16> {
        let mut value = 0_u16;
        for _ in 0..4 {
            let digit = match self.next() {
                Some(b'0'..=b'9') => u16::from(self.bytes[self.offset - 1] - b'0'),
                Some(b'a'..=b'f') => u16::from(self.bytes[self.offset - 1] - b'a' + 10),
                Some(b'A'..=b'F') => u16::from(self.bytes[self.offset - 1] - b'A' + 10),
                _ => return Err(self.invalid_json()),
            };
            value = (value << 4) | digit;
        }
        Ok(value)
    }

    fn read_number(&mut self) -> YrsEngineResult<()> {
        let start = self.offset;
        let negative = self.consume(b'-');
        match self.peek() {
            Some(b'0') => {
                self.offset += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(self.invalid_json());
                }
            }
            Some(b'1'..=b'9') => {
                self.offset += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.offset += 1;
                }
            }
            _ => return Err(self.invalid_json()),
        }

        let mut fractional = false;
        if self.consume(b'.') {
            fractional = true;
            self.read_digits()?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            fractional = true;
            self.offset += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            self.read_digits()?;
        }

        let source = std::str::from_utf8(&self.bytes[start..self.offset])
            .map_err(|_| self.invalid_json())?;
        let accepted = if fractional || negative {
            source.parse::<f64>().is_ok_and(|number| number.is_finite())
        } else {
            match source.parse::<u64>() {
                Ok(number) => number <= i64::MAX as u64,
                Err(_) => source.parse::<f64>().is_ok_and(|number| number.is_finite()),
            }
        };
        if accepted {
            Ok(())
        } else {
            Err(self.invalid_json())
        }
    }

    fn read_digits(&mut self) -> YrsEngineResult<()> {
        let start = self.offset;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.offset += 1;
        }
        if self.offset == start {
            Err(self.invalid_json())
        } else {
            Ok(())
        }
    }

    fn read_literal(&mut self, literal: &[u8]) -> YrsEngineResult<()> {
        let end = self
            .offset
            .checked_add(literal.len())
            .ok_or_else(|| self.invalid_json())?;
        if self.bytes.get(self.offset..end) == Some(literal) {
            self.offset = end;
            Ok(())
        } else {
            Err(self.invalid_json())
        }
    }

    fn charge_work(&mut self, amount: usize) -> YrsEngineResult<()> {
        self.work = self
            .work
            .checked_add(amount)
            .ok_or_else(|| self.invalid_json())?;
        let limit = raw_storage_work_limit(self.limits);
        if self.work > limit {
            return Err(
                YrsEngineError::limit("DOCUMENT_LIMIT_EXCEEDED", limit, self.work).with_details(
                    json!({
                        "field": "encodedState",
                        "phase": "updatePreflight",
                        "dimension": "work"
                    }),
                ),
            );
        }
        Ok(())
    }

    fn charge_string_payload(&mut self, amount: usize) -> YrsEngineResult<()> {
        self.string_payload = self
            .string_payload
            .checked_add(amount)
            .ok_or_else(|| self.invalid_json())?;
        if self.string_payload > self.limits.max_input_bytes {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                self.limits.max_input_bytes,
                self.string_payload,
            )
            .with_details(json!({
                "field": "encodedState",
                "phase": "updatePreflight",
                "dimension": "jsonStringBytes"
            })));
        }
        Ok(())
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.offset += 1;
        }
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.offset += 1;
            true
        } else {
            false
        }
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.offset += 1;
        Some(byte)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn invalid_json(&self) -> YrsEngineError {
        YrsEngineError::new(
            "COLLABORATION_DECODE_FAILED",
            "encoded snapshot JSON content failed structural preflight",
        )
        .with_details(json!({
            "field": "encodedState",
            "phase": "updatePreflight",
            "reason": "invalidJson"
        }))
    }
}

#[cfg(test)]
#[path = "update_preflight/tests.rs"]
mod tests;
