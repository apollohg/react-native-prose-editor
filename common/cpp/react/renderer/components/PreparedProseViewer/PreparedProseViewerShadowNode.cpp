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
  if (!std::isfinite(maximumWidth) || maximumWidth < 0 ||
      !std::isfinite(pointScaleFactor) || pointScaleFactor <= 0 ||
      !measurementsManager_) {
    return {};
  }

  const auto physicalWidth = std::round(maximumWidth * pointScaleFactor);
  if (!std::isfinite(physicalWidth) || physicalWidth < 0) {
    return {};
  }

  const auto effectiveWidth = physicalWidth / pointScaleFactor;
  const auto& state = getStateData();
  return measurementsManager_->measure(
      getSurfaceId(),
      getConcreteProps(),
      effectiveWidth,
      pointScaleFactor,
      state.attachmentRevision,
      state.nativeFontRevision);
}

} // namespace facebook::react
