#import <Foundation/Foundation.h>
#import <UIKit/UIKit.h>

#if __has_include("ReactNativeProseEditor-Swift.h")
#import "ReactNativeProseEditor-Swift.h"
#else
#error "ReactNativeProseEditor Swift compatibility header is unavailable; verify the pod module name and consumer codegen"
#endif

#include <react/renderer/components/PreparedProseViewer/PreparedProseMeasurementsManager.h>

#include <optional>

namespace facebook::react {
namespace {

NSString *StringFromStdString(const std::string &value) {
  return [[NSString alloc] initWithBytes:value.data()
                                  length:value.size()
                                encoding:NSUTF8StringEncoding] ?: @"";
}

NSString *OptionalStringFromStdString(const std::optional<std::string> &value) {
  return value ? StringFromStdString(*value) : nil;
}

NSString *SourceKind(const PreparedProseViewerProps &props) {
  return props.sourceKind == PreparedProseViewerSourceKind::Html ? @"html" : @"json";
}

} // namespace

void PreparedProseMeasurementsManager::bindLeaseLifecycle(
    SurfaceId surfaceId,
    Tag componentTag,
    uint64_t leaseHandle,
    const std::shared_ptr<PreparedProseViewerLeaseLifecycle>& leaseLifecycle) const {
  if (leaseHandle == 0 || !leaseLifecycle || !leaseLifecycle->isActive()) return;
  [[PREPPreparedProseLayoutRegistry sharedRegistry]
      registerFabricLeaseSurfaceId:static_cast<int64_t>(surfaceId)
                    componentTag:static_cast<int64_t>(componentTag)
                     leaseHandle:leaseHandle];
  leaseLifecycle->bindTerminalCleanup([
      surfaceId, componentTag, leaseHandle] {
    [[PREPPreparedProseLayoutRegistry sharedRegistry]
        releaseFabricLeaseSurfaceId:static_cast<int64_t>(surfaceId)
                  componentTag:static_cast<int64_t>(componentTag)
                   leaseHandle:leaseHandle];
  });
}

Size PreparedProseMeasurementsManager::measure(
    SurfaceId surfaceId,
    Tag componentTag,
    const PreparedProseViewerProps &props,
    Float effectiveWidth,
    Float pointScaleFactor,
    uint64_t attachmentRevision,
    uint64_t nativeFontRevision,
    double nativeFontScale,
    int32_t userInterfaceStyle,
    int32_t accessibilityContrast,
    uint64_t fontEnvironmentRevision,
    uint64_t leaseHandle,
    const std::shared_ptr<PreparedProseViewerLeaseLifecycle>& leaseLifecycle) const {
  @autoreleasepool {
    const auto size = [[PREPPreparedProseLayoutRegistry sharedRegistry]
        measureSurfaceId:static_cast<int64_t>(surfaceId)
          componentTag:static_cast<int64_t>(componentTag)
           leaseHandle:leaseHandle
            sourceKind:SourceKind(props)
                    source:StringFromStdString(props.source)
                configJSON:StringFromStdString(props.configJson)
                 themeJSON:OptionalStringFromStdString(props.themeJson)
           imagePolicyJSON:OptionalStringFromStdString(props.imagePolicyJson)
            imagesEnabled:props.imagesEnabled
      collapsesWhenEmpty:props.collapsesWhenEmpty
       attachmentRevision:attachmentRevision
       nativeFontRevision:nativeFontRevision
          nativeFontScale:nativeFontScale
  fontEnvironmentRevision:(fontEnvironmentRevision)
       userInterfaceStyle:userInterfaceStyle
     accessibilityContrast:accessibilityContrast
              widthPoints:effectiveWidth
                     scale:pointScaleFactor];
    // A release may race a Yoga callback already in flight. The shadow state
    // owns the lifecycle flag, so remove only this exact handle after the
    // synchronous registry call if the callback lost that race.
    if (leaseHandle != 0 && leaseLifecycle && !leaseLifecycle->isActive()) {
      [[PREPPreparedProseLayoutRegistry sharedRegistry]
          releaseFabricLeaseSurfaceId:static_cast<int64_t>(surfaceId)
                    componentTag:static_cast<int64_t>(componentTag)
                     leaseHandle:leaseHandle];
    }
    return {static_cast<Float>(size.width), static_cast<Float>(size.height)};
  }
}

void PreparedProseMeasurementsManager::prepareFinalLayout(
    SurfaceId,
    Tag,
    const PreparedProseViewerProps&,
    int32_t,
    int32_t,
    int32_t,
    Float,
    uint64_t,
    uint64_t,
    double,
    int32_t,
    int32_t,
    uint64_t,
    uint64_t,
    const std::shared_ptr<PreparedProseViewerLeaseLifecycle>&) const {
  // The iOS component view consumes Fabric's final content frame directly.
}

} // namespace facebook::react
