import CoreText
import Foundation
import UIKit
import XCTest

extension PreparedProseLayoutTests {
    func testBenchmarkCensusIncludesOnlyObservedLiveArtifacts() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 3)
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func layout(_ key: ProseLayoutKey, bytes: Int) -> PreparedProseLayout {
            PreparedProseLayout(
                key: key,
                size: CGSize(width: 160, height: 20),
                blocks: [],
                retainedBytes: bytes
            )
        }

        let directKey = key("direct")
        let pendingKey = key("pending")
        let mountedKey = key("mounted")
        let completedKey = key("completed")
        let replacementKey = key("replacement")
        let oversizedKey = key("oversized")
        let rejectedKey = key("rejected")
        let surface = FabricSurfaceToken(surfaceId: 88, componentTag: 880)

        cache.registerDirectMount("benchmark-direct", layout: layout(directKey, bytes: 2))
        _ = try cache.value(for: pendingKey, fabricSurface: surface, fabricLeaseHandle: 1) {
            layout(pendingKey, bytes: 2)
        }
        _ = try cache.value(for: mountedKey, fabricSurface: surface, fabricLeaseHandle: 2) {
            layout(mountedKey, bytes: 2)
        }
        XCTAssertTrue(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: mountedKey.generationIdentity,
            widthPixels: mountedKey.widthPixels,
            displayScale: 2,
            leaseHandle: 2
        ) != nil)
        _ = try cache.value(for: completedKey) { layout(completedKey, bytes: 1) }

        cache.beginBenchmarkCensus()
        _ = try cache.value(for: directKey) { XCTFail("direct owner should satisfy the lookup"); return layout(directKey, bytes: 2) }
        _ = try cache.value(for: pendingKey, fabricSurface: surface, fabricLeaseHandle: 1) { XCTFail("pending owner should satisfy the lookup"); return layout(pendingKey, bytes: 2) }
        _ = try cache.value(for: mountedKey, fabricSurface: surface, fabricLeaseHandle: 2) { XCTFail("mounted owner should satisfy the lookup"); return layout(mountedKey, bytes: 2) }
        _ = try cache.value(for: completedKey) { XCTFail("completed entry should satisfy the lookup"); return layout(completedKey, bytes: 1) }
        XCTAssertThrowsError(try cache.value(for: rejectedKey) { throw NSError(domain: "benchmark", code: 1) })
        XCTAssertEqual(
            Set(cache.endBenchmarkCensus()),
            Set([directKey, pendingKey, mountedKey, completedKey])
        )

        // Re-observe the completed entry, then evict it. The replacement is
        // live; the old key, a rejected lookup, and an oversized unowned
        // artifact are not.
        cache.beginBenchmarkCensus()
        _ = try cache.value(for: completedKey) { XCTFail("completed entry should satisfy the lookup"); return layout(completedKey, bytes: 1) }
        _ = try cache.value(for: replacementKey) { layout(replacementKey, bytes: 1) }
        XCTAssertThrowsError(try cache.value(for: rejectedKey) { throw NSError(domain: "benchmark", code: 2) })
        _ = try cache.value(for: oversizedKey) { layout(oversizedKey, bytes: 4) }
        XCTAssertEqual(
            Set(cache.endBenchmarkCensus()),
            Set([replacementKey]),
            "the observed completed key was evicted; rejected and oversized unowned lookups never became resident"
        )
    }

    func testBenchmarkCensusRetainsSeededLiveKeysWithoutReverseLookup() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 8)
        let key = ProseLayoutKey(
            semanticKey: "reverse-seeded",
            widthPixels: 320,
            themeDigest: "theme",
            nativeFontRevision: 0,
            fontEnvironmentRevision: 0,
            displayScale: 2,
            attachmentRevision: 0,
            generationIdentity: "reverse-seeded",
            semanticGenerationIdentity: "reverse-seeded"
        )
        let artifact = PreparedProseLayout(
            key: key,
            size: CGSize(width: 160, height: 20),
            blocks: [],
            retainedBytes: 1
        )

        cache.beginBenchmarkCensus()
        _ = try cache.value(for: key) { artifact }
        let forwardKeys = cache.endBenchmarkCensus()

        cache.beginBenchmarkCensus(seeding: forwardKeys)

        XCTAssertEqual(
            Set(cache.endBenchmarkCensus()),
            Set([key]),
            "a reverse pass must retain a prime-pass key that remains live even when UIKit does not rebind its only cell"
        )
    }

    func testReleasingDirectMountEnforcesBudgetBeforePublishingUnmountedCache() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 3)
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func artifact(_ key: ProseLayoutKey) -> PreparedProseLayout {
            PreparedProseLayout(
                key: key,
                size: CGSize(width: 160, height: 20),
                blocks: [],
                retainedBytes: 2
            )
        }

        let directKey = key("direct")
        let unmountedKey = key("unmounted")
        let directArtifact = try cache.value(for: directKey) { artifact(directKey) }
        cache.registerDirectMount("direct-owner", layout: directArtifact)
        _ = try cache.value(for: unmountedKey) { artifact(unmountedKey) }

        cache.releaseDirectMount("direct-owner")

        XCTAssertLessThanOrEqual(cache.retainedBytesForTesting, 3)
        XCTAssertEqual(cache.countForTesting, 1)
    }

    func testLayoutCacheRepeatedHitsPreserveExactLRURecency() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 2)
        var builds: [String: Int] = [:]
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func resolve(_ name: String) throws -> PreparedProseLayout {
            let layoutKey = key(name)
            return try cache.value(for: layoutKey) {
                builds[name, default: 0] += 1
                return PreparedProseLayout(
                    key: layoutKey,
                    size: CGSize(width: 160, height: 20),
                    blocks: [],
                    retainedBytes: 1
                )
            }
        }

        _ = try resolve("first")
        _ = try resolve("second")
        for _ in 0..<128 { _ = try resolve("first") }
        _ = try resolve("third")
        _ = try resolve("first")
        _ = try resolve("second")

        XCTAssertEqual(builds["first"], 1, "repeated hits must keep the entry most-recent")
        XCTAssertEqual(builds["second"], 2, "the untouched oldest entry must be evicted first")
        XCTAssertEqual(cache.countForTesting, 2)
    }

    func testLayoutCacheCompactsConsumedAccessPrefixBeforeRepeatedHitsGrowStorage() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 1)
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func resolve(_ name: String) throws -> PreparedProseLayout {
            let layoutKey = key(name)
            return try cache.value(for: layoutKey) {
                PreparedProseLayout(
                    key: layoutKey,
                    size: CGSize(width: 160, height: 20),
                    blocks: [],
                    retainedBytes: 1
                )
            }
        }

        for index in 0..<256 { _ = try resolve("evicted-\(index)") }
        for _ in 0..<128 { _ = try resolve("evicted-255") }

        XCTAssertLessThanOrEqual(
            cache.accessOrderTokenCountForTesting,
            65,
            "a large consumed prefix must not remain allocated while the live entry is repeatedly touched"
        )
    }

    func testMountedFabricLeaseReleaseEvictsReturnedCompletedEntryBeforePublishing() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 3)
        let surface = FabricSurfaceToken(surfaceId: 1, componentTag: 1)
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func artifact(_ layoutKey: ProseLayoutKey) -> PreparedProseLayout {
            PreparedProseLayout(
                key: layoutKey,
                size: CGSize(width: 160, height: 20),
                blocks: [],
                retainedBytes: 2
            )
        }

        let mountedKey = key("mounted")
        let completedKey = key("completed")
        _ = try cache.value(for: mountedKey, fabricSurface: surface, fabricLeaseHandle: 1) { artifact(mountedKey) }
        XCTAssertNotNil(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: mountedKey.generationIdentity,
            widthPixels: mountedKey.widthPixels,
            displayScale: 2,
            leaseHandle: 1
        ))
        _ = try cache.value(for: completedKey) { artifact(completedKey) }

        cache.releaseLease(for: surface, generationIdentity: mountedKey.generationIdentity, leaseHandle: 1)

        XCTAssertEqual(cache.retainedBytesForTesting, 2)
        XCTAssertEqual(cache.countForTesting, 1)
        var mountedRebuilds = 0
        _ = try cache.value(for: mountedKey) {
            mountedRebuilds += 1
            return artifact(mountedKey)
        }
        XCTAssertEqual(mountedRebuilds, 1)
    }

    func testFabricMountReplacementEvictsStaleMountedCompletedEntryBeforePublishing() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 4)
        let surface = FabricSurfaceToken(surfaceId: 1, componentTag: 1)
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func artifact(_ layoutKey: ProseLayoutKey, retainedBytes: Int) -> PreparedProseLayout {
            PreparedProseLayout(
                key: layoutKey,
                size: CGSize(width: 160, height: 20),
                blocks: [],
                retainedBytes: retainedBytes
            )
        }

        let staleMountedKey = key("stale-mounted")
        let replacementKey = key("replacement")
        let completedKey = key("completed")
        _ = try cache.value(for: staleMountedKey, fabricSurface: surface, fabricLeaseHandle: 1) { artifact(staleMountedKey, retainedBytes: 3) }
        XCTAssertNotNil(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: staleMountedKey.generationIdentity,
            widthPixels: staleMountedKey.widthPixels,
            displayScale: 2,
            leaseHandle: 1
        ))
        _ = try cache.value(for: replacementKey, fabricSurface: surface, fabricLeaseHandle: 1) { artifact(replacementKey, retainedBytes: 1) }
        _ = try cache.value(for: completedKey) { artifact(completedKey, retainedBytes: 3) }

        XCTAssertNotNil(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: replacementKey.generationIdentity,
            widthPixels: replacementKey.widthPixels,
            displayScale: 2,
            leaseHandle: 1
        ))

        XCTAssertEqual(cache.retainedBytesForTesting, 4)
        XCTAssertEqual(cache.countForTesting, 2)
        var staleRebuilds = 0
        _ = try cache.value(for: staleMountedKey) {
            staleRebuilds += 1
            return artifact(staleMountedKey, retainedBytes: 3)
        }
        XCTAssertEqual(staleRebuilds, 1)
    }

    func testReplacingDirectMountEvictsReturnedCompletedEntryBeforePublishing() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 3)
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func artifact(_ layoutKey: ProseLayoutKey) -> PreparedProseLayout {
            PreparedProseLayout(
                key: layoutKey,
                size: CGSize(width: 160, height: 20),
                blocks: [],
                retainedBytes: 2
            )
        }

        let replacedKey = key("replaced")
        let replacementKey = key("replacement")
        let completedKey = key("completed")
        let replaced = try cache.value(for: replacedKey) { artifact(replacedKey) }
        let replacement = try cache.value(for: replacementKey) { artifact(replacementKey) }
        cache.registerDirectMount("first-owner", layout: replaced)
        cache.registerDirectMount("second-owner", layout: replacement)
        _ = try cache.value(for: completedKey) { artifact(completedKey) }

        cache.registerDirectMount("first-owner", layout: replacement)

        XCTAssertEqual(cache.retainedBytesForTesting, 4)
        XCTAssertEqual(cache.countForTesting, 2)
        var replacedRebuilds = 0
        _ = try cache.value(for: replacedKey) {
            replacedRebuilds += 1
            return artifact(replacedKey)
        }
        XCTAssertEqual(replacedRebuilds, 1)
    }

    func testLayoutCacheDeduplicatesMixedOwnersAndEvictsAfterDirectRelease() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 4)
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func artifact(_ layoutKey: ProseLayoutKey) -> PreparedProseLayout {
            PreparedProseLayout(
                key: layoutKey,
                size: CGSize(width: 160, height: 20),
                blocks: [],
                retainedBytes: 2
            )
        }

        let directKey = key("direct")
        let pendingKey = key("pending")
        let mountedKey = key("mounted")
        let completedKey = key("completed")
        let surface = FabricSurfaceToken(surfaceId: 99, componentTag: 990)

        let direct = try cache.value(for: directKey) { artifact(directKey) }
        cache.registerDirectMount("direct-owner", layout: direct)
        _ = try cache.value(for: pendingKey, fabricSurface: surface, fabricLeaseHandle: 1) { artifact(pendingKey) }
        _ = try cache.value(for: mountedKey, fabricSurface: surface, fabricLeaseHandle: 2) { artifact(mountedKey) }
        XCTAssertNotNil(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: mountedKey.generationIdentity,
            widthPixels: mountedKey.widthPixels,
            displayScale: 2,
            leaseHandle: 2
        ))
        _ = try cache.value(for: completedKey) { artifact(completedKey) }

        XCTAssertEqual(cache.retainedBytesForTesting, 8, "each identity is retained once across completed, pending, mounted, and direct roles")
        XCTAssertEqual(cache.pendingLeaseCountForTesting, 1)
        XCTAssertEqual(cache.mountedLeaseCountForTesting, 1)
        XCTAssertEqual(cache.countForTesting, 4)

        cache.releaseDirectMount("direct-owner")

        XCTAssertEqual(cache.retainedBytesForTesting, 6)
        XCTAssertEqual(cache.pendingLeaseCountForTesting, 1)
        XCTAssertEqual(cache.mountedLeaseCountForTesting, 1)
        XCTAssertEqual(cache.countForTesting, 3, "direct release must evict its newly budgeted oldest completed entry")
        var directRebuilds = 0
        _ = try cache.value(for: directKey) {
            directRebuilds += 1
            return artifact(directKey)
        }
        XCTAssertEqual(directRebuilds, 1)
    }

    func testExactPendingLeasesSurviveBytePressureUntilTheirOwnersAcquire() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 1)
        let key = ProseLayoutKey(
            semanticKey: String(repeating: "a", count: 64),
            widthPixels: 320,
            themeDigest: "theme",
            nativeFontRevision: 0,
            fontEnvironmentRevision: 0,
            displayScale: 2,
            attachmentRevision: 0,
            generationIdentity: "shared",
            semanticGenerationIdentity: "shared"
        )
        let artifact = PreparedProseLayout(
            key: key,
            size: CGSize(width: 160, height: 20),
            blocks: [],
            retainedBytes: 80
        )
        let surface = FabricSurfaceToken(surfaceId: 88, componentTag: 880)
        let mounted: UInt64 = 1
        let firstPending: UInt64 = 2
        let secondPending: UInt64 = 3
        let preferred: UInt64 = 4

        _ = try cache.value(for: key, fabricSurface: surface, fabricLeaseHandle: mounted) { artifact }
        XCTAssertTrue(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: key.generationIdentity,
            widthPixels: key.widthPixels,
            displayScale: 2,
            leaseHandle: mounted
        ) === artifact)
        for handle in [firstPending, secondPending, preferred] {
            _ = try cache.value(for: key, fabricSurface: surface, fabricLeaseHandle: handle) {
                XCTFail("A live immutable artifact must be reused.")
                return artifact
            }
        }

        XCTAssertEqual(cache.pendingLeaseCountForTesting, 3)
        XCTAssertEqual(cache.mountedLeaseCountForTesting, 1)
        XCTAssertTrue(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: key.generationIdentity,
            widthPixels: key.widthPixels,
            displayScale: 2,
            leaseHandle: firstPending
        ) === artifact)
        XCTAssertTrue(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: key.generationIdentity,
            widthPixels: key.widthPixels,
            displayScale: 2,
            leaseHandle: secondPending
        ) === artifact)
        XCTAssertTrue(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: key.generationIdentity,
            widthPixels: key.widthPixels,
            displayScale: 2,
            leaseHandle: preferred
        ) === artifact)
    }

}
