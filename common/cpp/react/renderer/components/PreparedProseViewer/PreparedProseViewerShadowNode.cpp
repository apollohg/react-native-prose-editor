#include "PreparedProseViewerShadowNode.h"

#include <cmath>

namespace facebook::react {

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

  const auto hasUsableMeasurement = std::isfinite(maximumWidth) && maximumWidth > 0 &&
      std::isfinite(pointScaleFactor) && pointScaleFactor > 0;
  const auto effectiveWidth = hasUsableMeasurement
      ? std::round(maximumWidth * pointScaleFactor) / pointScaleFactor
      : maximumWidth;
  const auto& props = getConcreteProps();
  const auto& state = getStateData();
  return measurementsManager_->measure(
      getSurfaceId(),
      props,
      effectiveWidth,
      pointScaleFactor,
      state.attachmentRevision,
      state.nativeFontRevision,
      props.fontEnvironmentRevision > 0
          ? static_cast<uint64_t>(props.fontEnvironmentRevision)
          : 0);
}

} // namespace facebook::react
