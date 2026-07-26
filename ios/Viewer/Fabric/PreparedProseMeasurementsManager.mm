#import <Foundation/Foundation.h>

#import "ReactNativeProseEditor-Swift.h"

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

Size PreparedProseMeasurementsManager::measure(
    SurfaceId /*surfaceId*/,
    const PreparedProseViewerProps &props,
    Float effectiveWidth,
    Float pointScaleFactor,
    uint64_t /*attachmentRevision*/,
    uint64_t /*nativeFontRevision*/) const {
  @autoreleasepool {
    const auto size = [[PREPPreparedProseLayoutRegistry sharedRegistry]
        measureSourceKind:SourceKind(props)
                    source:StringFromStdString(props.source)
                configJSON:StringFromStdString(props.configJson)
                 themeJSON:OptionalStringFromStdString(props.themeJson)
           imagePolicyJSON:OptionalStringFromStdString(props.imagePolicyJson)
            imagesEnabled:props.imagesEnabled
      collapsesWhenEmpty:props.collapsesWhenEmpty
              widthPoints:effectiveWidth
                     scale:pointScaleFactor];
    return {static_cast<Float>(size.width), static_cast<Float>(size.height)};
  }
}

} // namespace facebook::react
