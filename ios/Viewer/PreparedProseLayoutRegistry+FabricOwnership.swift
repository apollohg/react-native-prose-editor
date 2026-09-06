import CryptoKit
import Foundation
import UIKit

extension PreparedProseLayoutRegistry {
    func beginFabricMeasure(_ generation: FabricGenerationToken) -> Bool {
        compiledCondition.lock()
        guard isFabricLeaseActiveLocked(generation) else {
            compiledCondition.unlock()
            return false
        }
        var state = fabricMeasurementsInFlight[generation] ?? FabricMeasurementState()
        guard !state.cancelled else {
            compiledCondition.unlock()
            return false
        }
        state.count += 1
        fabricMeasurementsInFlight[generation] = state
        compiledCondition.unlock()
        return true
    }

    func endFabricMeasure(_ generation: FabricGenerationToken) {
        compiledCondition.lock()
        guard var state = fabricMeasurementsInFlight[generation] else {
            compiledCondition.unlock()
            return
        }
        let remaining = max(0, state.count - 1)
        if remaining == 0 {
            fabricMeasurementsInFlight.removeValue(forKey: generation)
        } else {
            state.count = remaining
            fabricMeasurementsInFlight[generation] = state
        }
        compiledCondition.unlock()
    }

    func isFabricLeaseActive(_ generation: FabricGenerationToken) -> Bool {
        compiledCondition.lock()
        defer { compiledCondition.unlock() }
        return isFabricLeaseActiveLocked(generation)
    }

    /// Caller must hold `compiledCondition`.
    func isFabricLeaseActiveLocked(_ generation: FabricGenerationToken) -> Bool {
        guard generation.leaseHandle != 0,
              let lease = fabricLeaseStates[FabricLeaseOwner(
                  surface: generation.surface,
                  leaseHandle: generation.leaseHandle
              )],
              lease.active,
              lease.permittedGenerationIdentity == nil ||
                  lease.permittedGenerationIdentity == generation.generationIdentity
        else { return false }
        return !(fabricMeasurementsInFlight[generation]?.cancelled ?? false)
    }

    /// Caller must hold `compiledCondition`.
    func cancelFabricMeasurementLocked(_ generation: FabricGenerationToken) {
        guard var state = fabricMeasurementsInFlight[generation] else { return }
        state.cancelled = true
        fabricMeasurementsInFlight[generation] = state
    }

    private func retireStaleFabricLease(
        _ generation: FabricGenerationToken,
        widthPixels: Int,
        scale: CGFloat
    ) {
        _ = layoutCache.releasePendingLease(
            for: generation.surface,
            generationIdentity: generation.generationIdentity,
            widthPixels: widthPixels,
            displayScale: scale,
            leaseHandle: generation.leaseHandle
        )
    }

    func discardCancelledFabricMeasurement(
        _ generation: FabricGenerationToken,
        widthPixels: Int,
        scale: CGFloat
    ) {
        retireStaleFabricLease(generation, widthPixels: widthPixels, scale: scale)
        // Release can win after sidecar begin but before cache publication.
        // This exact handle cannot clear a concurrently-created replacement.
        FabricAttachmentSidecars.remove(
            generation.surface,
            leaseHandle: generation.leaseHandle
        )
    }

    func retainFabricGenerationOwnership(
        _ generation: FabricGenerationToken,
        document: ViewerDocument,
        request: ProseViewerRequest
    ) -> Bool {
        compiledCondition.lock()
        guard isFabricLeaseActiveLocked(generation) else {
            compiledCondition.unlock()
            return false
        }
        documentsByFabricGeneration[generation] = document
        failuresByFabricGeneration.removeValue(forKey: generation)
        _ = preparedThemeLocked(for: request, generation: generation)
        fabricOwnershipRevisions[generation, default: 0] &+= 1
        compiledCondition.unlock()
        return true
    }

    func retainFabricGenerationFailure(
        _ generation: FabricGenerationToken,
        error: Error
    ) -> Bool {
        compiledCondition.lock()
        guard isFabricLeaseActiveLocked(generation) else {
            compiledCondition.unlock()
            return false
        }
        failuresByFabricGeneration[generation] = error
        documentsByFabricGeneration.removeValue(forKey: generation)
        fabricOwnershipRevisions[generation, default: 0] &+= 1
        compiledCondition.unlock()
        return true
    }

}
