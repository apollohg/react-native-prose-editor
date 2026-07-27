#pragma once

#include <cmath>
#include <atomic>
#include <cstdint>
#include <memory>

#ifdef ANDROID
#include <folly/dynamic.h>
#endif

namespace facebook::react {

/// Shared by every Fabric state clone for one viewer incarnation. The shadow
/// node checks this before handing work to a platform registry, while a
/// recycled component view deactivates it before releasing its exact lease.
/// Its lifetime is bounded by the state snapshots that reference it.
class PreparedProseViewerLeaseLifecycle final {
 public:
  bool isActive() const {
    return active_.load(std::memory_order_acquire);
  }

  void deactivate() {
    active_.store(false, std::memory_order_release);
  }

 private:
  std::atomic<bool> active_{true};
};

struct PreparedProseViewerState final {
  uint64_t attachmentRevision{0};
  uint64_t nativeFontRevision{0};
  // A native Dynamic Type invalidation carries the scale that caused it.
  // The JS revision can remain unchanged, so it cannot select this snapshot.
  double nativeFontScale{1.0};
  // Opaque, native-owned Fabric handoff incarnation. The shadow node mints it
  // once and component views must pass this exact value back to the registry.
  uint64_t leaseHandle{0};
  // This is deliberately C++-only state. Dynamic platform updates retain the
  // previous lifecycle when they omit the native-owned handle.
  std::shared_ptr<PreparedProseViewerLeaseLifecycle> leaseLifecycle{};

#ifdef ANDROID
  PreparedProseViewerState() = default;

  PreparedProseViewerState(
      const PreparedProseViewerState& previousState,
      const folly::dynamic& data)
      : attachmentRevision(revisionValue(data, "attachmentRevision")),
        nativeFontRevision(revisionValue(data, "nativeFontRevision")),
        nativeFontScale(scaleValue(data, "nativeFontScale")),
        leaseHandle(handleValue(data, "leaseHandle", previousState.leaseHandle)),
        leaseLifecycle(lifecycleValue(previousState, leaseHandle)) {}

  folly::dynamic getDynamic() const {
    return folly::dynamic::object("attachmentRevision", attachmentRevision)(
        "nativeFontRevision", nativeFontRevision)(
        "nativeFontScale", nativeFontScale)(
        "leaseHandle", static_cast<int64_t>(leaseHandle));
  }

 private:
  static uint64_t revisionValue(const folly::dynamic& data, const char* key) {
    const auto value = data.getDefault(key, 0).asInt();
    return value < 0 ? 0 : static_cast<uint64_t>(value);
  }

  static double scaleValue(const folly::dynamic& data, const char* key) {
    const auto value = data.getDefault(key, 1.0).asDouble();
    return std::isfinite(value) && value > 0.0 ? value : 1.0;
  }

  static uint64_t handleValue(
      const folly::dynamic& data,
      const char* key,
      uint64_t fallback) {
    if (data.count(key) == 0) {
      return fallback;
    }
    const auto value = data.getDefault(key, 0).asInt();
    return value < 0 ? 0 : static_cast<uint64_t>(value);
  }

  static std::shared_ptr<PreparedProseViewerLeaseLifecycle> lifecycleValue(
      const PreparedProseViewerState& previousState,
      uint64_t handle) {
    if (handle == 0) {
      return nullptr;
    }
    if (handle == previousState.leaseHandle && previousState.leaseLifecycle) {
      return previousState.leaseLifecycle;
    }
    return std::make_shared<PreparedProseViewerLeaseLifecycle>();
  }
#endif
};

} // namespace facebook::react
