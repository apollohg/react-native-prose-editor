#pragma once

#include "PreparedProseMeasurementsManager.h"
#include "PreparedProseViewerState.h"

#include <react/renderer/components/ReactNativeProseEditorSpec/EventEmitters.h>
#include <react/renderer/components/ReactNativeProseEditorSpec/Props.h>
#include <react/renderer/components/view/ConcreteViewShadowNode.h>

namespace facebook::react {

class ComponentDescriptor;

extern const char PreparedProseViewerComponentName[];

class PreparedProseViewerShadowNode final
    : public ConcreteViewShadowNode<
          PreparedProseViewerComponentName,
          PreparedProseViewerProps,
          PreparedProseViewerEventEmitter,
          PreparedProseViewerState> {
 public:
  using ConcreteViewShadowNode::ConcreteViewShadowNode;

  static ShadowNodeTraits BaseTraits();

  static ConcreteStateData initialStateData(
      const Props::Shared& props,
      const ShadowNodeFamily::Shared& family,
      const ComponentDescriptor& componentDescriptor);

  void setMeasurementsManager(
      const std::shared_ptr<PreparedProseMeasurementsManager>&
          measurementsManager);

  Size measureContent(
      const LayoutContext& layoutContext,
      const LayoutConstraints& layoutConstraints) const override;

 private:
  std::shared_ptr<PreparedProseMeasurementsManager> measurementsManager_;
};

} // namespace facebook::react
