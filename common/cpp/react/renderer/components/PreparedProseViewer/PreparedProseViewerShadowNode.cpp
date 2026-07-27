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
  static std::atomic<uint64_t> next{0};
  auto handle = next.fetch_add(1, std::memory_order_relaxed) + 1;
  // Zero is the explicit "no committed handoff" sentinel in component state.
  if (handle == 0) {
    handle = next.fetch_add(1, std::memory_order_relaxed) + 1;
  }
  return handle;
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
