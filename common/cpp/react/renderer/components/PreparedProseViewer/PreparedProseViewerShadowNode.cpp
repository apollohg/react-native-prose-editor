#include "PreparedProseViewerShadowNode.h"

#include <react/renderer/core/LayoutContext.h>

#include <cmath>
#include <atomic>
#include <limits>

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
  // The Android JNI contract is a signed jlong.  Stop rather than wrapping:
  // aliasing a still-live owner would be worse than declining a theoretical
  // 9e18th viewer incarnation.
  static std::atomic<int64_t> next{0};
  auto current = next.load(std::memory_order_relaxed);
  while (current > 0 && current < std::numeric_limits<int64_t>::max()) {
    if (next.compare_exchange_weak(
            current, current + 1, std::memory_order_relaxed,
            std::memory_order_relaxed)) {
      return static_cast<uint64_t>(current + 1);
    }
  }
  if (current == 0 && next.compare_exchange_strong(
          current, 1, std::memory_order_relaxed, std::memory_order_relaxed)) {
    return 1;
  }
  return 0;
}

} // namespace

extern const char PreparedProseViewerComponentName[] = "PreparedProseViewer";

ShadowNodeTraits PreparedProseViewerShadowNode::BaseTraits() {
  auto traits = ConcreteViewShadowNode::BaseTraits();
  traits.set(ShadowNodeTraits::Trait::LeafYogaNode);
  traits.set(ShadowNodeTraits::Trait::MeasurableYogaNode);
  return traits;
}

PreparedProseViewerShadowNode::ConcreteStateData
PreparedProseViewerShadowNode::initialStateData(
    const Props::Shared& /*props*/,
    const ShadowNodeFamily::Shared& /*family*/,
    const ComponentDescriptor& /*componentDescriptor*/) {
  PreparedProseViewerState state;
  state.leaseHandle = NextFabricLeaseHandle();
  state.leaseLifecycle = std::make_shared<PreparedProseViewerLeaseLifecycle>();
  return state;
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
  // The committed state mints this once, before Yoga's first measurement.
  // A delayed callback from a released incarnation may still calculate an
  // unowned size, but it must never recreate a Fabric lease or sidecar.
  const auto leaseHandle = state.leaseLifecycle && state.leaseLifecycle->isActive()
      ? state.leaseHandle
      : 0;
  if (leaseHandle != 0) {
    measurementsManager_->bindLeaseLifecycle(
        getSurfaceId(), getTag(), leaseHandle, state.leaseLifecycle);
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
      leaseHandle,
      state.leaseLifecycle);
}

} // namespace facebook::react
