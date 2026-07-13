use serde_json::json;

use crate::boundary::ResourceLimits;

use super::{YrsEngineError, YrsEngineResult};

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
}

impl<'a> PreflightReader<'a> {
    fn new(bytes: &'a [u8], limits: &'a ResourceLimits) -> Self {
        Self {
            bytes,
            offset: 0,
            limits,
            work: 0,
            payload: 0,
        }
    }

    fn read_update(&mut self) -> YrsEngineResult<()> {
        let clients = self.read_var_u32()? as usize;
        self.require_declared_count(clients, 3)?;
        self.charge_work(clients)?;
        for _ in 0..clients {
            let blocks = self.read_var_u32()? as usize;
            self.require_declared_count(blocks, 1)?;
            self.read_var_u32()?; // client
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
                self.read_string()?;
                Ok(1)
            }
            6 => {
                self.read_string()?;
                self.read_string()?;
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
            self.read_var_u32()?;
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
        self.read_var_u32()?;
        self.read_var_u32()?;
        Ok(())
    }

    fn read_string(&mut self) -> YrsEngineResult<&'a str> {
        let bytes = self.read_buffer()?;
        std::str::from_utf8(bytes).map_err(|_| self.decode_error("invalidUtf8"))
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
        if count > self.limits.max_document_nodes {
            // No separate decoded-update item ceiling exists. Reuse the
            // configurable node ceiling so compact encodings cannot amplify
            // into attacker-sized Yrs collection allocations before the
            // derived document is available for ordinary node validation.
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                self.limits.max_document_nodes,
                count,
            )
            .with_details(json!({
                "field": "encodedState",
                "phase": "updatePreflight",
                "dimension": "collectionItems"
            })));
        }
        Ok(())
    }

    fn charge_work(&mut self, amount: usize) -> YrsEngineResult<()> {
        self.work = self
            .work
            .checked_add(amount)
            .ok_or_else(|| self.decode_error("workOverflow"))?;
        let limit = self.limits.max_document_nodes;
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

#[cfg(test)]
mod tests {
    use super::preflight_update_v1;
    use crate::boundary::ResourceLimits;
    use yrs::{Doc, ReadTxn, StateVector, Text, Transact};

    fn any_update(any: &[u8]) -> Vec<u8> {
        let mut update = vec![1, 1, 1, 0, 8, 1, 0, 1];
        update.extend_from_slice(any);
        update.push(0); // empty delete set
        update
    }

    fn nested_array(depth: usize) -> Vec<u8> {
        let mut any = Vec::with_capacity(depth * 2 + 1);
        for _ in 0..depth {
            any.extend_from_slice(&[117, 1]);
        }
        any.push(126);
        any
    }

    #[test]
    fn any_depth_accepts_the_exact_boundary_and_rejects_one_over() {
        let limits = ResourceLimits {
            max_document_depth: 8,
            ..ResourceLimits::default()
        };

        preflight_update_v1(&any_update(&nested_array(7)), &limits).unwrap();
        let error = preflight_update_v1(&any_update(&nested_array(8)), &limits).unwrap_err();

        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(8));
        assert_eq!(error.actual, Some(9));
        assert_eq!(
            error.details,
            Some(serde_json::json!({
                "field": "encodedState",
                "phase": "updatePreflight",
                "dimension": "anyDepth"
            }))
        );
    }

    #[test]
    fn impossible_declared_collection_lengths_fail_without_allocating() {
        let limits = ResourceLimits::default();
        for (name, bytes) in [
            ("clients", vec![127]),
            ("itemAny", vec![1, 1, 1, 0, 8, 1, 0, 127]),
            ("array", any_update(&[117, 127])),
            ("map", any_update(&[118, 127])),
            ("deleteRanges", vec![0, 1, 1, 127]),
        ] {
            let error = preflight_update_v1(&bytes, &limits).unwrap_err();
            assert_eq!(error.code, "COLLABORATION_DECODE_FAILED", "{name}");
            assert_eq!(
                error.details,
                Some(serde_json::json!({
                    "field": "encodedState",
                    "phase": "updatePreflight",
                    "reason": "declaredLength"
                })),
                "{name}"
            );
        }
    }

    #[test]
    fn collection_allocations_and_aggregate_work_use_exact_node_boundaries() {
        let collection_limits = ResourceLimits {
            max_document_nodes: 4,
            ..ResourceLimits::default()
        };
        let error = preflight_update_v1(
            &any_update(&[117, 5, 126, 126, 126, 126, 126]),
            &collection_limits,
        )
        .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(4));
        assert_eq!(error.actual, Some(5));
        assert_eq!(
            error.details,
            Some(serde_json::json!({
                "field": "encodedState",
                "phase": "updatePreflight",
                "dimension": "collectionItems"
            }))
        );

        let work_limits = ResourceLimits {
            max_document_nodes: 5,
            ..ResourceLimits::default()
        };
        preflight_update_v1(&any_update(&[117, 2, 126, 126]), &work_limits).unwrap();
        let error =
            preflight_update_v1(&any_update(&[117, 3, 126, 126, 126]), &work_limits).unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(5));
        assert_eq!(error.actual, Some(6));
        assert_eq!(
            error.details,
            Some(serde_json::json!({
                "field": "encodedState",
                "phase": "updatePreflight",
                "dimension": "work"
            }))
        );
    }

    #[test]
    fn truncation_invalid_utf8_and_trailing_bytes_fail_preflight() {
        let limits = ResourceLimits::default();
        for bytes in [vec![1], vec![1, 1, 1, 0, 4, 1, 0, 1, 255, 0], vec![0, 0, 0]] {
            let error = preflight_update_v1(&bytes, &limits).unwrap_err();
            assert_eq!(error.code, "COLLABORATION_DECODE_FAILED");
            assert_eq!(error.details.as_ref().unwrap()["phase"], "updatePreflight");
        }
    }

    #[test]
    fn accepts_empty_and_generated_standard_update_v1() {
        let limits = ResourceLimits::default();
        preflight_update_v1(&[0, 0], &limits).unwrap();

        let doc = Doc::new();
        doc.get_or_insert_text("text")
            .push(&mut doc.transact_mut(), "hello");
        let update = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        preflight_update_v1(&update, &limits).unwrap();
    }
}
