#pragma once

#include <cstdint>
#include <memory>

#include <react/renderer/components/ReactNativeProseEditorSpec/Props.h>
#include <react/renderer/core/LayoutConstraints.h>
#include <react/renderer/core/ReactPrimitives.h>
#include <react/renderer/core/SurfaceId.h>
#include <react/utils/ContextContainer.h>

namespace facebook::react {

class PreparedProseMeasurementsManager {
 public:
  explicit PreparedProseMeasurementsManager(
      const ContextContainer::Shared& contextContainer)
      : contextContainer_(contextContainer) {}

  Size measure(
      SurfaceId surfaceId,
      Tag componentTag,
      const PreparedProseViewerProps& props,
      Float effectiveWidth,
      Float pointScaleFactor,
      uint64_t attachmentRevision,
      uint64_t nativeFontRevision,
      uint64_t fontEnvironmentRevision) const;

 private:
  const ContextContainer::Shared contextContainer_;
};

} // namespace facebook::react
