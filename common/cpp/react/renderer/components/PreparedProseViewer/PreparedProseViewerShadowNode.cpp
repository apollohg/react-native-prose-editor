#include "PreparedProseViewerShadowNode.h"

#include <react/renderer/core/LayoutContext.h>

#include <cmath>
#include <atomic>
#include <limits>
#include <mutex>
#include <unordered_map>

namespace facebook::react {

namespace {

bool HasRepresentablePhysicalWidth(Float width, Float scale) {
  if (!std::isfinite(width) || width <= 0 || !std::isfinite(scale) ||
      scale <= 0) {
    return false;
  }
  const double physicalWidth = static_cast<double>(width) * static_cast<double>(scale);
  if (!std::isfinite(physicalWidth) || physicalWidth <= 0) {
    return false;
  }
  const double roundedWidth = std::round(physicalWidth);
  const double largestConvertible = std::nextafter(
      static_cast<double>(std::numeric_limits<long long>::max()), 0.0);
  return std::isfinite(roundedWidth) && roundedWidth > 0 &&
      roundedWidth <= largestConvertible;
}

uint64_t FontEnvironmentRevision(const PreparedProseViewerProps &props) {
  const double value = static_cast<double>(props.fontEnvironmentRevision);
  const double largestConvertible = std::nextafter(
      static_cast<double>(std::numeric_limits<uint64_t>::max()), 0.0);
  return std::isfinite(value) && value > 0 && value <= largestConvertible
      ? static_cast<uint64_t>(value)
      : 0;
}

uint64_t NextFabricLeaseHandle() {
  static std::atomic<uint64_t> next{0};
  auto handle = next.fetch_add(1, std::memory_order_relaxed) + 1;
  // Zero is the explicit "no committed handoff" sentinel in component state.
  if (handle == 0) {
    handle = next.fetch_add(1, std::memory_order_relaxed) + 1;
  }
  return handle;
}

struct PendingLeaseHandle {
  const PreparedProseViewerState* stateData{nullptr};
  uint64_t handle{0};
};

std::mutex& PendingLeaseHandleMutex() {
  static std::mutex mutex;
  return mutex;
}

std::unordered_map<const ShadowNodeFamily*, PendingLeaseHandle>&
PendingLeaseHandles() {
  static std::unordered_map<const ShadowNodeFamily*, PendingLeaseHandle> handles;
  return handles;
}

uint64_t PendingLeaseHandleFor(
    const ShadowNodeFamily& family,
    const PreparedProseViewerState& state) {
  std::lock_guard<std::mutex> lock(PendingLeaseHandleMutex());
  auto& pending = PendingLeaseHandles()[&family];
  if (pending.stateData != &state || pending.handle == 0) {
    pending.stateData = &state;
    pending.handle = NextFabricLeaseHandle();
  }
  return pending.handle;
}

void ClearPendingLeaseHandle(const ShadowNodeFamily& family) {
  std::lock_guard<std::mutex> lock(PendingLeaseHandleMutex());
  PendingLeaseHandles().erase(&family);
}

} // namespace

extern const char PreparedProseViewerComponentName[] = "PreparedProseViewer";

ShadowNodeTraits PreparedProseViewerShadowNode::BaseTraits() {
  auto traits = ConcreteViewShadowNode::BaseTraits();
  traits.set(ShadowNodeTraits::Trait::LeafYogaNode);
  traits.set(ShadowNodeTraits::Trait::MeasurableYogaNode);
  return traits;
}

void PreparedProseViewerShadowNode::setMeasurementsManager(
    const std::shared_ptr<PreparedProseMeasurementsManager>&
        measurementsManager) {
  ensureUnsealed();
  measurementsManager_ = measurementsManager;
}

Size PreparedProseViewerShadowNode::measureContent(
    const LayoutContext& layoutContext,
    const LayoutConstraints& layoutConstraints) const {
  const auto maximumWidth = layoutConstraints.maximumSize.width;
  const auto pointScaleFactor = layoutContext.pointScaleFactor;
  if (!measurementsManager_) {
    return {};
  }

  const auto hasUsableMeasurement =
      HasRepresentablePhysicalWidth(maximumWidth, pointScaleFactor);
  const auto effectiveWidth = hasUsableMeasurement
      ? std::round(maximumWidth * pointScaleFactor) / pointScaleFactor
      : maximumWidth;
  const auto& props = getConcreteProps();
  const auto& state = getStateData();
  auto leaseHandle = state.leaseHandle;
  const auto& family = getFamily();
  const auto concreteState =
      std::static_pointer_cast<const ConcreteState>(state_);
  if (!hasUsableMeasurement) {
    // An invalid-width callback owns no new handoff. If it belongs to an
    // existing incarnation, retire only that exact handle and make the next
    // valid Yoga measure mint a fresh one.
    if (leaseHandle != 0) {
      ClearPendingLeaseHandle(family);
      concreteState->updateState(
          [leaseHandle](const ConcreteState::Data& current)
              -> ConcreteState::SharedData {
            if (current.leaseHandle != leaseHandle) {
              return nullptr;
            }
            auto next = current;
            next.leaseHandle = 0;
            return std::make_shared<const ConcreteState::Data>(next);
          });
    }
  } else if (leaseHandle == 0) {
    leaseHandle = PendingLeaseHandleFor(family, state);
    concreteState->updateState(
        [leaseHandle](const ConcreteState::Data& current)
            -> ConcreteState::SharedData {
          // A competing native state update won this commit. Its handle is
          // authoritative; this delayed measurement cannot publish one.
          if (current.leaseHandle != 0) {
            return nullptr;
          }
          auto next = current;
          next.leaseHandle = leaseHandle;
          return std::make_shared<const ConcreteState::Data>(next);
        });
  } else {
    ClearPendingLeaseHandle(family);
  }
  return measurementsManager_->measure(
      getSurfaceId(),
      getTag(),
      props,
      effectiveWidth,
      pointScaleFactor,
      state.attachmentRevision,
      state.nativeFontRevision,
      state.nativeFontScale,
      FontEnvironmentRevision(props),
      leaseHandle);
}

} // namespace facebook::react
