import CoreText
import Foundation
import UIKit
import XCTest

extension PreparedProseLayoutTests {
    func testStaleFabricMountMissPreservesNewerPendingWidthAndMountedArtifact() {
        let registry = PreparedProseLayoutRegistry(
            byteBudget: 1,
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                PreparedProseLayout(
                    key: key,
                    size: CGSize(width: width, height: 20),
                    blocks: [],
                    retainedBytes: Int(width)
                )
            }
        )
        let request = request()
        let surface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        let mountedView = PreparedProseDrawingView(frame: .zero)

        let mounted = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface)
        XCTAssertTrue(install(request, in: mountedView, surface: surface, registry: registry))
        let replacement = registry.measure(request: request, widthPoints: 140, scale: 2, fabricSurface: surface)

        registry.releaseFabricMountMiss(
            FabricGenerationToken(
                surface: surface,
                generationIdentity: canonicalFabricGenerationIdentity(request, registry: registry)
            ),
            widthPoints: 160,
            scale: 2
        )

        XCTAssertTrue(mountedView.layout === mounted)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 1)
        XCTAssertTrue(install(request, in: mountedView, surface: surface, registry: registry, width: 140))
        XCTAssertTrue(mountedView.layout === replacement)
    }

    func testFabricLeaseHandlePreventsStaleLifecycleFromTouchingReplacement() {
        let registry = PreparedProseLayoutRegistry(
            byteBudget: 1,
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                PreparedProseLayout(
                    key: key,
                    size: CGSize(width: width, height: 20),
                    blocks: [],
                    retainedBytes: Int(width)
                )
            }
        )
        let request = request()
        let surface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        let generationIdentity = canonicalFabricGenerationIdentity(request, registry: registry)
        let h1 = FabricGenerationToken(surface: surface, generationIdentity: generationIdentity, leaseHandle: 101)
        let h2 = FabricGenerationToken(surface: surface, generationIdentity: generationIdentity, leaseHandle: 202)
        registry.registerFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: h1.leaseHandle)
        registry.registerFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: h2.leaseHandle)

        _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: h1.leaseHandle)
        _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: h2.leaseHandle)

        // A delayed H1 mount can consume only H1; it cannot select H2 by
        // surface/generation or by any newest-epoch policy.
        XCTAssertTrue(install(request, in: PreparedProseDrawingView(frame: .zero), surface: surface, registry: registry, leaseHandle: h1.leaseHandle))
        registry.releaseFabricMountMiss(h1, widthPoints: 160, scale: 2)
        registry.releaseFabricGeneration(h1)
        _ = registry.measure(request: request, widthPoints: 0, scale: 2, fabricSurface: surface, fabricLeaseHandle: h1.leaseHandle)

        let replacement = PreparedProseDrawingView(frame: .zero)
        XCTAssertTrue(install(request, in: replacement, surface: surface, registry: registry, leaseHandle: h2.leaseHandle))
        XCTAssertTrue(registry.hasFabricGenerationOwnershipForTesting(h2))

        // Width replacement stays within H2 and atomically replaces only H2's
        // mounted ownership.
        let replacementLayout = registry.measure(
            request: request,
            widthPoints: 140,
            scale: 2,
            fabricSurface: surface,
            fabricLeaseHandle: h2.leaseHandle
        )
        XCTAssertTrue(install(request, in: replacement, surface: surface, registry: registry, width: 140, leaseHandle: h2.leaseHandle))
        XCTAssertTrue(replacement.layout === replacementLayout)
    }

    func testFabricLeaseHandleBridgeStaticContract() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        func source(_ path: String) throws -> String {
            try String(contentsOf: root.appendingPathComponent(path), encoding: .utf8)
        }

        let state = try source("common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerState.h")
        let shadow = try source("common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerShadowNode.cpp")
        let manager = try source("ios/Viewer/Fabric/PreparedProseMeasurementsManager.mm")
        let registry = try source("ios/Viewer/PreparedProseLayoutRegistry.swift")
            + source("ios/Viewer/PreparedProseLayoutRegistry+Fabric.swift")
            + source("ios/Viewer/PreparedProseLayoutRegistry+FabricOwnership.swift")
        let component = try source("ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.mm")
        let cache = try source("ios/Viewer/PreparedProseLayoutCache.swift")
        let android = try source("android/src/main/jni/PreparedProseViewerMeasurementsManager.cpp")

        XCTAssertTrue(state.contains("uint64_t leaseHandle{0}"))
        XCTAssertTrue(state.contains("previousState.leaseHandle"))
        XCTAssertTrue(state.contains("if (data.count(key) == 0)"))
        XCTAssertTrue(state.contains("return fallback;"))
        XCTAssertTrue(state.contains("leaseLifecycle"))
        XCTAssertTrue(state.contains("~PreparedProseViewerLeaseLifecycle"))
        XCTAssertTrue(state.contains("bindTerminalCleanup"))
        XCTAssertTrue(shadow.contains("NextFabricLeaseHandle"))
        XCTAssertTrue(shadow.contains("initialStateData"))
        XCTAssertTrue(shadow.contains("state.leaseHandle = NextFabricLeaseHandle()"))
        XCTAssertLessThan(
            shadow.range(of: "initialStateData")!.lowerBound,
            shadow.range(of: "measureContent")!.lowerBound
        )
        XCTAssertFalse(shadow.contains("updateState("))
        XCTAssertFalse(shadow.contains("PendingLeaseHandles"))
        XCTAssertFalse(shadow.contains("ShadowNodeFamily*"))
        XCTAssertTrue(manager.contains("leaseHandle:leaseHandle"))
        XCTAssertTrue(manager.contains("bindLeaseLifecycle"))
        XCTAssertTrue(manager.contains("registerFabricLeaseSurfaceId"))
        XCTAssertTrue(manager.contains("!leaseLifecycle->isActive()"))
        XCTAssertTrue(android.contains("beginNativeMeasure"))
        XCTAssertTrue(android.contains("static_cast<int64_t>(leaseHandle)"))
        XCTAssertTrue(registry.contains("leaseHandle: UInt64"))
        XCTAssertTrue(registry.contains("releaseFabricLeaseSurfaceId"))
        XCTAssertTrue(registry.contains("permittedGenerationIdentity"))
        XCTAssertTrue(registry.contains("activateFabricGeneration"))
        XCTAssertTrue(registry.contains("releaseFabricInvalidMeasurement"))
        XCTAssertFalse(registry.contains("retiredFabricLeaseHandles"))
        XCTAssertTrue(component.contains("installCachedLayoutInDrawingView"))
        XCTAssertTrue(component.contains("const auto leaseHandle = LeaseHandle(_viewerState)"))
        XCTAssertTrue(component.contains("leaseHandle:leaseHandle"))
        XCTAssertTrue(component.contains("beginNewGenerationTerminatingCurrentLease:NO"))
        XCTAssertTrue(component.contains("releaseFabricOwnershipTerminatingLease:YES"))
        XCTAssertTrue(component.contains("if (terminal) {"))
        XCTAssertTrue(component.contains("DeactivateLease(_viewerState, stateLeaseHandle)"))
        XCTAssertTrue(component.contains("releaseFabricMountMissSurfaceId"))
        XCTAssertTrue(component.contains("activateFabricGenerationSurfaceId"))
        XCTAssertTrue(component.contains("leaseHandle:_ownedLeaseHandle"))
        XCTAssertTrue(cache.contains("$0.leaseHandle == leaseHandle"))
        XCTAssertTrue(cache.contains("fabricGenerations(for surface"))
        XCTAssertFalse(cache.contains(".max(by: { $0.epoch < $1.epoch })"))
    }

    func testFabricStateLifecycleUsesOneHandleAcrossOrdinaryRevisionsStaticContract() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let component = try String(
            contentsOf: root.appendingPathComponent("ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.mm"),
            encoding: .utf8
        )
        let shadow = try String(
            contentsOf: root.appendingPathComponent("common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerShadowNode.cpp"),
            encoding: .utf8
        )
        XCTAssertTrue(component.contains("beginNewGenerationTerminatingCurrentLease:NO"))
        XCTAssertTrue(component.contains("leaseChanged"))
        XCTAssertTrue(component.contains("beginNewGenerationTerminatingCurrentLease:leaseChanged"))
        XCTAssertTrue(component.contains("releaseFabricOwnershipTerminatingLease:YES"))
        XCTAssertTrue(shadow.contains("std::numeric_limits<int64_t>::max()"))
        XCTAssertTrue(shadow.contains("for (;;)"))
        XCTAssertTrue(shadow.contains("compare_exchange_weak"))
        XCTAssertTrue(shadow.contains("Restart from that value"))
        XCTAssertTrue(shadow.contains("return 0;"))
    }

    func testFabricComponentRetriesInstallationAfterWindowAttachmentStaticContract() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let component = try String(
            contentsOf: root.appendingPathComponent("ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.mm"),
            encoding: .utf8
        )
        let callback = try XCTUnwrap(component.range(of: "- (void)didMoveToWindow"))
        let installGate = try XCTUnwrap(
            component.range(
                of: "[self installMeasuredArtifactIfAttached]",
                range: callback.lowerBound ..< component.endIndex
            )
        )

        XCTAssertLessThan(callback.lowerBound, installGate.lowerBound)
    }

    func testNeverMountedFabricStateBindsExactTerminalCleanupStaticContract() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let state = try String(
            contentsOf: root.appendingPathComponent("common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerState.h"),
            encoding: .utf8
        )
        let bridge = try String(
            contentsOf: root.appendingPathComponent("ios/Viewer/Fabric/PreparedProseMeasurementsManager.mm"),
            encoding: .utf8
        )
        XCTAssertTrue(state.contains("final snapshot dies"))
        XCTAssertTrue(bridge.contains("releaseFabricLeaseSurfaceId"))
        XCTAssertTrue(bridge.contains("leaseLifecycle->bindTerminalCleanup"))
    }

    func testAndroidNativeStateKeepsExactLeaseHandleStaticContract() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let manager = try String(
            contentsOf: root.appendingPathComponent("android/src/main/java/com/apollohg/editor/viewer/PreparedProseViewerManager.kt"),
            encoding: .utf8
        )
        let cache = try String(
            contentsOf: root.appendingPathComponent("android/src/main/java/com/apollohg/editor/viewer/PreparedProseLayoutCache.kt"),
            encoding: .utf8
        )
        let jni = try String(
            contentsOf: root.appendingPathComponent("android/src/main/jni/PreparedProseViewerMeasurementsManager.cpp"),
            encoding: .utf8
        )
        XCTAssertTrue(manager.contains("stringOrNull(\"leaseHandle\")?.toLongOrNull()"))
        XCTAssertTrue(manager.contains("FabricLeaseHandleBridge.currentHandle()"))
        XCTAssertTrue(cache.contains("pendingLeases"))
        XCTAssertTrue(cache.contains("mountedLeases"))
        XCTAssertFalse(cache.contains("completed[mountIndex"))
        XCTAssertTrue(jni.contains("getStaticMethod<void(jlong)>(\"beginNativeMeasure\")"))
        XCTAssertTrue(jni.contains("registerNativeLease"))
        XCTAssertTrue(jni.contains("finalizeNativeLease"))
        XCTAssertTrue(jni.contains("global_ref<facebook::jni::JClass>"))
        XCTAssertFalse(jni.contains("alias_ref<facebook::jni::JClass>"))
        XCTAssertTrue(jni.contains("make_global"))
        XCTAssertTrue(jni.contains("processLifetime"))
        XCTAssertTrue(jni.contains("process-lifetime"))
        XCTAssertFalse(jni.contains("bridge.reset()"))
        XCTAssertTrue(jni.contains("facebook::jni::ThreadScope"))
        XCTAssertTrue(jni.contains("Every object allocation, class lookup, and Java invocation below can run"))
        XCTAssertTrue(jni.contains("Still inside ThreadScope"))
        XCTAssertTrue(jni.contains("std::to_string(static_cast<int64_t>(leaseHandle))"))
        XCTAssertTrue(jni.contains("folly::dynamic localData"))
        XCTAssertTrue(jni.contains("beginNativeMeasure_(bridgeClass_"))
        XCTAssertTrue(jni.contains("endNativeMeasure_(bridgeClass_)"))
    }

}
