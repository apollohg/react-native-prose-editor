import UIKit

enum NativeEditorDestroyReservationResult: Equatable {
    case reserved
    case alreadyInProgress
    case unavailable
}

final class NativeEditorViewRegistry {
    static let shared = NativeEditorViewRegistry()

    private final class RegisteredView {
        weak var view: NativeEditorExpoView?
        let order: UInt64

        init(view: NativeEditorExpoView, order: UInt64) {
            self.view = view
            self.order = order
        }
    }

    private var viewsByEditorId: [UInt64: [RegisteredView]] = [:]
    private var nextRegistrationOrder: UInt64 = 0
    private var activeEditorIds: Set<UInt64> = []
    private var destroyingEditorIds: Set<UInt64> = []
    var onFinalizeDestroyForTesting: ((UInt64) throws -> Void)?

    private init() {}

    func markEditorCreated(editorId: UInt64) {
        guard editorId != 0 else { return }
        _ = performOnMain {
            activeEditorIds.insert(editorId)
        }
    }

    func rebaseAfterRemoteCommit(editorId: UInt64) {
        guard editorId != 0 else { return }
        DispatchQueue.main.async { [weak self] in
            self?.applyRemoteCommitRefresh(editorId: editorId)
        }
    }

    func applyRemoteCommitRefresh(editorId: UInt64) {
        dispatchPrecondition(condition: .onQueue(.main))
        liveRegisteredViews(editorId: editorId).forEach { $0.view?.applyRemoteCommitRefresh() }
    }

    func isDestroyed(editorId: UInt64) -> Bool {
        guard editorId != 0 else { return false }
        return performOnMain {
            if destroyingEditorIds.contains(editorId) { return true }
            if activeEditorIds.contains(editorId) {
                return false
            }
            return EditorV2Registry.adapter(forLegacyId: editorId) == nil
        }
    }

    @discardableResult
    func register(editorId: UInt64, view: NativeEditorExpoView) -> Bool {
        guard editorId != 0 else { return false }
        return performOnMain {
            guard !destroyingEditorIds.contains(editorId) else { return false }
            if !activeEditorIds.contains(editorId) {
                guard EditorV2Registry.adapter(forLegacyId: editorId) != nil else {
                    return false
                }
                activeEditorIds.insert(editorId)
            }
            var views = liveRegisteredViews(editorId: editorId)
            guard !views.contains(where: { $0.view === view }) else { return true }
            nextRegistrationOrder &+= 1
            views.append(RegisteredView(view: view, order: nextRegistrationOrder))
            viewsByEditorId[editorId] = views
            return true
        }
    }

    func unregister(editorId: UInt64, view: NativeEditorExpoView) {
        guard editorId != 0 else { return }
        performOnMain {
            let views = liveRegisteredViews(editorId: editorId).filter { $0.view !== view }
            if views.isEmpty {
                viewsByEditorId.removeValue(forKey: editorId)
            } else {
                viewsByEditorId[editorId] = views
            }
        }
    }

    func nativeOwnerReleased(editorId: UInt64, by view: NativeEditorExpoView) {
        guard editorId != 0 else { return }
        performOnMain {
            guard !destroyingEditorIds.contains(editorId) else { return }
            let survivor = liveRegisteredViews(editorId: editorId)
                .reversed()
                .compactMap(\.view)
                .first { $0 !== view && $0.window != nil }
            survivor?.claimNativeOwnershipAndCatchUp(editorId: editorId)
        }
    }

    func invalidateDestroyedEditor(editorId: UInt64) {
        guard reserveDestroy(editorId: editorId) else { return }
        finalizeDestroy(editorId: editorId)
    }

    /// Install the reversible, in-flight destroy gate before crossing the
    /// FFI boundary. The active view state remains intact until finalization
    /// so a retryable destroy can roll back without reconstructing bindings.
    func acquireDestroyReservation(editorId: UInt64) -> NativeEditorDestroyReservationResult {
        guard editorId != 0 else { return .unavailable }
        return performOnMain {
            guard !destroyingEditorIds.contains(editorId) else { return .alreadyInProgress }
            guard activeEditorIds.contains(editorId)
                || EditorV2Registry.adapter(forLegacyId: editorId) != nil
            else {
                return .unavailable
            }
            destroyingEditorIds.insert(editorId)
            return .reserved
        }
    }

    @discardableResult
    func reserveDestroy(editorId: UInt64) -> Bool {
        acquireDestroyReservation(editorId: editorId) == .reserved
    }

    func rollbackDestroy(editorId: UInt64) {
        guard editorId != 0 else { return }
        _ = performOnMain {
            destroyingEditorIds.remove(editorId)
        }
    }

    func isDestroyReserved(editorId: UInt64) -> Bool {
        guard editorId != 0 else { return false }
        return performOnMain {
            destroyingEditorIds.contains(editorId)
        }
    }

    func finalizeDestroy(editorId: UInt64) {
        guard editorId != 0 else { return }
        performOnMain {
            guard destroyingEditorIds.contains(editorId) else { return }
            invokeDestroyTestingHook(onFinalizeDestroyForTesting, editorId: editorId)
            activeEditorIds.remove(editorId)
            let views = viewsByEditorId.removeValue(forKey: editorId)?.compactMap(\.view) ?? []
            views.forEach { $0.handleEditorDestroyed(editorId) }
            destroyingEditorIds.remove(editorId)
        }
    }

    func destroy(editorId: UInt64, operation: () -> Void) {
        guard reserveDestroy(editorId: editorId) else { return }
        operation()
        finalizeDestroy(editorId: editorId)
    }

    func prepareForCommandJSON(editorId: UInt64) -> String {
        let prepare = { () -> String in
            if self.destroyingEditorIds.contains(editorId) {
                return Self.commandPreparationJSON(ready: false, blockedReason: "destroying")
            }
            if !self.activeEditorIds.contains(editorId),
               EditorV2Registry.adapter(forLegacyId: editorId) == nil
            {
                return Self.commandPreparationJSON(ready: false, blockedReason: "destroyed")
            }
            let views = self.liveRegisteredViews(editorId: editorId).compactMap(\.view)
            guard let view = views.reversed().first(where: {
                $0.ownsNativeBinding(editorId: editorId)
            }) ?? views.last else {
                self.viewsByEditorId.removeValue(forKey: editorId)
                return Self.commandPreparationJSON(ready: true)
            }
            return view.prepareForEditorCommandJSON()
        }

        return performOnMain(prepare)
    }

    private func liveRegisteredViews(editorId: UInt64) -> [RegisteredView] {
        let views = viewsByEditorId[editorId]?.filter { $0.view != nil } ?? []
        if views.isEmpty {
            viewsByEditorId.removeValue(forKey: editorId)
        } else {
            viewsByEditorId[editorId] = views
        }
        return views.sorted { $0.order < $1.order }
    }

    static func commandPreparationJSON(
        ready: Bool,
        updateJSON: String? = nil,
        blockedReason: String? = nil
    ) -> String {
        var payload: [String: Any] = ["ready": ready]
        if let updateJSON {
            payload["updateJSON"] = updateJSON
        }
        if let blockedReason {
            payload["blockedReason"] = blockedReason
        }
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            if let blockedReason {
                return ready
                    ? "{\"ready\":true,\"blockedReason\":\"\(blockedReason)\"}"
                    : "{\"ready\":false,\"blockedReason\":\"\(blockedReason)\"}"
            }
            return ready ? "{\"ready\":true}" : "{\"ready\":false}"
        }
        return json
    }

    private func performOnMain<T>(_ work: () -> T) -> T {
        if Thread.isMainThread {
            return work()
        }
        return DispatchQueue.main.sync(execute: work)
    }
}
