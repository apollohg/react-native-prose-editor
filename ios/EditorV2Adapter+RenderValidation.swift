import Foundation

extension EditorV2Adapter {
    /// One view-facing update plus the document's scalar extent (the lenient
    /// `UInt32.max` doc→scalar mapping, used to clamp transient-IME
    /// positions the way the legacy engine did).
    struct EditorV2DerivedUpdate {
        let updateJSON: String
        let scalarLength: UInt32
    }

    static func uint32Field(_ object: [String: Any], _ key: String) -> UInt32? {
        v2ExactUInt32(object[key] as? NSNumber)
    }

    struct AtomicRenderSnapshot {
        let atomicRenderJSON: String
        let viewUpdateJSON: String
        let documentRevision: UInt64
        let stateRevision: UInt64
        let scalarLength: UInt32
        let selection: (anchor: UInt32, head: UInt32)?
        let activeState: [String: Any]
        let historyState: (canUndo: Bool, canRedo: Bool)
        let documentIsEmpty: Bool
        let positionEpoch: UInt64?
    }

    static func exactBool(_ value: Any?) -> Bool? {
        guard let number = value as? NSNumber,
              CFGetTypeID(number) == CFBooleanGetTypeID()
        else {
            return nil
        }
        return number.boolValue
    }

    private static func finiteNumber(_ value: Any?) -> NSNumber? {
        guard let number = value as? NSNumber,
              CFGetTypeID(number) != CFBooleanGetTypeID(),
              number.doubleValue.isFinite
        else {
            return nil
        }
        return number
    }

    private static func hasOnlyKeys(_ object: [String: Any], _ allowed: Set<String>) -> Bool {
        Set(object.keys).isSubset(of: allowed)
    }

    private static func isValidJSONValue(_ value: Any) -> Bool {
        if value is NSNull || value is String || exactBool(value) != nil || finiteNumber(value) != nil {
            return true
        }
        if let array = value as? [Any] {
            return array.allSatisfy(isValidJSONValue)
        }
        if let object = value as? [String: Any] {
            return object.values.allSatisfy(isValidJSONValue)
        }
        return false
    }

    private static func isValidRenderMark(_ value: Any) -> Bool {
        if value is String { return true }
        guard let object = value as? [String: Any],
              object["type"] is String
        else {
            return false
        }
        return object.values.allSatisfy(isValidJSONValue)
    }

    private static func isValidListContext(_ value: Any) -> Bool {
        guard let object = value as? [String: Any],
              hasOnlyKeys(
                object,
                ["ordered", "index", "total", "start", "isFirst", "isLast", "kind", "checked"]
              ),
              exactBool(object["ordered"]) != nil,
              uint32Field(object, "index") != nil,
              uint32Field(object, "total") != nil,
              uint32Field(object, "start") != nil,
              exactBool(object["isFirst"]) != nil,
              exactBool(object["isLast"]) != nil
        else {
            return false
        }
        if let kind = object["kind"], !(kind is NSNull), !(kind is String) { return false }
        if let checked = object["checked"], !(checked is NSNull), exactBool(checked) == nil { return false }
        return true
    }

    private static func isValidMentionThemeSection(
        _ value: Any,
        stringKeys: Set<String>,
        extraKeys: Set<String>
    ) -> Bool {
        guard let object = value as? [String: Any] else { return false }
        guard hasOnlyKeys(object, stringKeys.union(mentionThemeNumberKeys).union(extraKeys)) else {
            return false
        }
        for key in stringKeys where object[key] != nil {
            guard object[key] is String else { return false }
        }
        for key in mentionThemeNumberKeys where object[key] != nil {
            guard finiteNumber(object[key]) != nil else { return false }
        }
        if let fontWeight = object["fontWeight"] {
            guard let fontWeight = fontWeight as? String,
                  mentionThemeFontWeights.contains(fontWeight)
            else {
                return false
            }
        }
        return true
    }

    private static func isValidMentionTheme(_ value: Any) -> Bool {
        guard let object = value as? [String: Any] else { return false }
        guard hasOnlyKeys(object, ["node", "suggestions"]) else { return false }

        if let node = object["node"] {
            guard isValidMentionThemeSection(
                node,
                stringKeys: mentionNodeStringKeys,
                extraKeys: ["fontWeight"]
            ) else {
                return false
            }
        }
        guard let suggestions = object["suggestions"] else { return true }
        guard isValidMentionThemeSection(
            suggestions,
            stringKeys: mentionSuggestionsStringKeys,
            extraKeys: ["option"]
        ) else {
            return false
        }
        guard let option = (suggestions as? [String: Any])?["option"] else { return true }
        return isValidMentionThemeSection(
            option,
            stringKeys: mentionOptionStringKeys,
            extraKeys: ["fontWeight"]
        )
    }

    private static func isValidRenderElement(_ value: Any) -> Bool {
        guard let object = value as? [String: Any],
              let type = object["type"] as? String
        else {
            return false
        }
        switch type {
        case "textRun":
            guard Set(object.keys) == ["type", "text", "marks"],
                  object["text"] is String,
                  let marks = object["marks"] as? [Any]
            else {
                return false
            }
            return marks.allSatisfy(isValidRenderMark)
        case "blockStart":
            guard hasOnlyKeys(object, ["type", "nodeType", "depth", "listContext"]),
                  object["nodeType"] is String,
                  uint32Field(object, "depth") != nil
            else {
                return false
            }
            return object["listContext"].map(isValidListContext) ?? true
        case "blockEnd":
            return Set(object.keys) == ["type"]
        case "voidInline":
            guard hasOnlyKeys(object, ["type", "nodeType", "docPos", "attrs"]),
                  object["nodeType"] is String,
                  uint32Field(object, "docPos") != nil
            else {
                return false
            }
            return object["attrs"].map { $0 is [String: Any] } ?? true
        case "voidBlock":
            guard hasOnlyKeys(object, ["type", "nodeType", "docPos", "attrs", "atomId"]),
                  object["nodeType"] is String,
                  uint32Field(object, "docPos") != nil,
                  object["atomId"].map({ $0 is String }) ?? true
            else {
                return false
            }
            return object["attrs"].map { $0 is [String: Any] } ?? true
        case "opaqueInlineAtom":
            guard hasOnlyKeys(
                      object,
                      ["type", "nodeType", "label", "docPos", "attrs", "mentionTheme"]
                  ),
                  object["nodeType"] is String,
                  object["label"] is String,
                  uint32Field(object, "docPos") != nil,
                  object["attrs"].map({ $0 is [String: Any] }) ?? true
            else {
                return false
            }
            return object["mentionTheme"].map(isValidMentionTheme) ?? true
        case "opaqueBlockAtom":
            guard hasOnlyKeys(object, ["type", "nodeType", "label", "docPos", "attrs"]),
                  object["nodeType"] is String,
                  object["label"] is String,
                  uint32Field(object, "docPos") != nil
            else {
                return false
            }
            return object["attrs"].map { $0 is [String: Any] } ?? true
        default:
            return false
        }
    }

    private static func isValidRenderBlocks(_ value: Any) -> Bool {
        guard let blocks = value as? [Any] else { return false }
        return blocks.allSatisfy { block in
            guard let elements = block as? [Any] else { return false }
            return elements.allSatisfy(isValidRenderElement)
        }
    }

    private static func isValidRenderPatch(_ value: Any) -> Bool {
        if value is NSNull { return true }
        guard let object = value as? [String: Any],
              Set(object.keys) == [
                "baseDocumentVersion",
                "startIndex",
                "deleteCount",
                "renderBlocks",
              ],
              uint64Field(object, "baseDocumentVersion") != nil,
              uint32Field(object, "startIndex") != nil,
              uint32Field(object, "deleteCount") != nil,
              let renderBlocks = object["renderBlocks"],
              isValidRenderBlocks(renderBlocks)
        else {
            return false
        }
        return true
    }

    private static func isBooleanRecord(_ value: Any?) -> Bool {
        guard let object = value as? [String: Any] else { return false }
        return object.values.allSatisfy { exactBool($0) != nil }
    }

    private static func isStringArray(_ value: Any?) -> Bool {
        guard let array = value as? [Any] else { return false }
        return array.allSatisfy { $0 is String }
    }

    private static func isValidActiveState(_ value: Any) -> Bool {
        guard let object = value as? [String: Any],
              Set(object.keys) == activeStateKeys,
              isBooleanRecord(object["marks"]),
              let markAttrs = object["markAttrs"] as? [String: Any],
              markAttrs.values.allSatisfy({ $0 is [String: Any] }),
              isBooleanRecord(object["nodes"]),
              isBooleanRecord(object["commands"]),
              isStringArray(object["allowedMarks"]),
              isStringArray(object["insertableNodes"])
        else {
            return false
        }
        return true
    }

    private static func scalarSelection(from value: Any) -> (anchor: UInt32, head: UInt32)? {
        guard let selection = value as? [String: Any],
              let type = selection["type"] as? String
        else {
            return nil
        }
        switch type {
        case "text":
            guard Set(selection.keys) == ["type", "anchor", "head", "anchorScalar", "headScalar"],
                  uint32Field(selection, "anchor") != nil,
                  uint32Field(selection, "head") != nil,
                  let anchor = uint32Field(selection, "anchorScalar"),
                  let head = uint32Field(selection, "headScalar")
            else {
                return nil
            }
            return (anchor, head)
        case "node":
            guard Set(selection.keys) == ["type", "pos", "posScalar"],
                  uint32Field(selection, "pos") != nil,
                  uint32Field(selection, "posScalar") != nil
            else {
                return nil
            }
            return nil
        case "all":
            return Set(selection.keys) == ["type"] ? nil : nil
        default:
            return nil
        }
    }

    private static func isValidSelection(_ value: Any) -> Bool {
        guard let selection = value as? [String: Any],
              let type = selection["type"] as? String
        else {
            return false
        }
        switch type {
        case "text":
            return scalarSelection(from: selection) != nil
        case "node":
            return Set(selection.keys) == ["type", "pos", "posScalar"]
                && uint32Field(selection, "pos") != nil
                && uint32Field(selection, "posScalar") != nil
        case "all":
            return Set(selection.keys) == ["type"]
        default:
            return false
        }
    }

    static func parseAtomicRenderSnapshot(_ json: String) -> AtomicRenderSnapshot? {
        guard let data = json.data(using: .utf8),
              var object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              Set(object.keys).isSubset(of: atomicRenderSnapshotKeys.union(["positionEpoch"])),
              atomicRenderSnapshotKeys.isSubset(of: Set(object.keys)),
              let renderBlocks = object["renderBlocks"],
              let renderPatch = object["renderPatch"],
              (isValidRenderBlocks(renderBlocks) && renderPatch is NSNull)
                || (renderBlocks is NSNull && !(renderPatch is NSNull) && isValidRenderPatch(renderPatch)),
              let selectionValue = object["selection"],
              isValidSelection(selectionValue),
              let activeState = object["activeState"] as? [String: Any],
              isValidActiveState(activeState),
              let history = object["historyState"] as? [String: Any],
              Set(history.keys) == ["canUndo", "canRedo"],
              let canUndo = exactBool(history["canUndo"]),
              let canRedo = exactBool(history["canRedo"]),
              let documentRevision = uint64Field(object, "documentVersion"),
              let stateRevision = uint64Field(object, "stateRevision"),
              let scalarLength = uint32Field(object, "scalarLength"),
              let documentIsEmpty = exactBool(object["documentIsEmpty"])
        else {
            return nil
        }

        let selection = scalarSelection(from: selectionValue)
        let positionEpoch: UInt64?
        if object.keys.contains("positionEpoch") {
            guard let value = object["positionEpoch"] as? String,
                  let parsed = UInt64(value), String(parsed) == value
            else {
                return nil
            }
            positionEpoch = parsed
        } else {
            positionEpoch = nil
        }
        object.removeValue(forKey: "positionEpoch")
        guard let atomicData = try? JSONSerialization.data(withJSONObject: object),
              let atomicRenderJSON = String(data: atomicData, encoding: .utf8)
        else {
            return nil
        }
        object.removeValue(forKey: "scalarLength")
        // documentIsEmpty stays in the view payload: the text view needs the
        // core's answer to decide whether to show its placeholder.
        guard let viewData = try? JSONSerialization.data(withJSONObject: object),
              let viewUpdateJSON = String(data: viewData, encoding: .utf8)
        else {
            return nil
        }
        return AtomicRenderSnapshot(
            atomicRenderJSON: atomicRenderJSON,
            viewUpdateJSON: viewUpdateJSON,
            documentRevision: documentRevision,
            stateRevision: stateRevision,
            scalarLength: scalarLength,
            selection: selection,
            activeState: activeState,
            historyState: (canUndo, canRedo),
            documentIsEmpty: documentIsEmpty,
            positionEpoch: positionEpoch
        )
    }

}
