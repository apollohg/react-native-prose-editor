#pragma once

#include "PreparedProseMeasurementsManager.h"
#include "PreparedProseViewerShadowNode.h"

#include <react/renderer/core/ConcreteComponentDescriptor.h>

namespace facebook::react {

class PreparedProseViewerComponentDescriptor final
    : public ConcreteComponentDescriptor<PreparedProseViewerShadowNode> {
 public:
  PreparedProseViewerComponentDescriptor(
      const ComponentDescriptorParameters& parameters)
      : ConcreteComponentDescriptor(parameters),
        measurementsManager_(std::make_shared<PreparedProseMeasurementsManager>(
            contextContainer_)) {}

  void adopt(ShadowNode& shadowNode) const override {
    ConcreteComponentDescriptor::adopt(shadowNode);
    auto& preparedProseViewerShadowNode =
        static_cast<PreparedProseViewerShadowNode&>(shadowNode);
    preparedProseViewerShadowNode.setMeasurementsManager(measurementsManager_);
  }

 private:
  const std::shared_ptr<PreparedProseMeasurementsManager> measurementsManager_;
};

} // namespace facebook::react
