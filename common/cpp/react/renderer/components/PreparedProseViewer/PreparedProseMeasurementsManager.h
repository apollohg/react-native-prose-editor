#pragma once

#include <cstdint>
#include <memory>

#include <react/renderer/components/ReactNativeProseEditorSpec/Props.h>
#include <react/renderer/components/PreparedProseViewer/PreparedProseViewerState.h>
#include <react/renderer/core/LayoutConstraints.h>
#include <react/renderer/core/ReactPrimitives.h>
#include <react/utils/ContextContainer.h>

namespace facebook::react {

class PreparedProseMeasurementsManager {
 public:
  explicit PreparedProseMeasurementsManager(
      const std::shared_ptr<const ContextContainer>& contextContainer)
      : contextContainer_(contextContainer) {}

  Size measure(
      SurfaceId surfaceId,
      Tag componentTag,
      const PreparedProseViewerProps& props,
      Float effectiveWidth,
      Float pointScaleFactor,
      uint64_t attachmentRevision,
      uint64_t nativeFontRevision,
      double nativeFontScale,
      int32_t userInterfaceStyle,
      int32_t accessibilityContrast,
      uint64_t fontEnvironmentRevision,
      uint64_t leaseHandle,
      const std::shared_ptr<PreparedProseViewerLeaseLifecycle>& leaseLifecycle) const;

  void prepareFinalLayout(
      SurfaceId surfaceId,
      Tag componentTag,
      const PreparedProseViewerProps& props,
      int32_t contentWidthPx,
      int32_t contentOriginXPx,
      int32_t contentOriginYPx,
      Float pointScaleFactor,
      uint64_t attachmentRevision,
      uint64_t nativeFontRevision,
      double nativeFontScale,
      int32_t userInterfaceStyle,
      int32_t accessibilityContrast,
      uint64_t fontEnvironmentRevision,
      uint64_t leaseHandle,
      const std::shared_ptr<PreparedProseViewerLeaseLifecycle>& leaseLifecycle) const;

  void bindLeaseLifecycle(
      SurfaceId surfaceId,
      Tag componentTag,
      uint64_t leaseHandle,
      const std::shared_ptr<PreparedProseViewerLeaseLifecycle>& leaseLifecycle) const;

 private:
  const std::shared_ptr<const ContextContainer> contextContainer_;
};

} // namespace facebook::react
