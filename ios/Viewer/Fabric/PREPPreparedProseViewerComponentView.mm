#import "PREPPreparedProseViewerComponentView.h"

#import "ReactNativeProseEditor-Swift.h"

#include <react/renderer/components/PreparedProseViewer/PreparedProseViewerComponentDescriptor.h>
#include <react/renderer/components/ReactNativeProseEditorSpec/EventEmitters.h>
#include <react/renderer/components/ReactNativeProseEditorSpec/Props.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>

#include <cmath>
#include <optional>

using namespace facebook::react;

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

@implementation PREPPreparedProseViewerComponentView {
  PREPPreparedProseDrawingView *_drawingView;
  std::shared_ptr<const PreparedProseViewerProps> _viewerProps;
  LayoutMetrics _layoutMetrics;
  NSString *_reportedErrorGeneration;
}

+ (ComponentDescriptorProvider)componentDescriptorProvider
{
  return concreteComponentDescriptorProvider<PreparedProseViewerComponentDescriptor>();
}

- (instancetype)initWithFrame:(CGRect)frame
{
  if (self = [super initWithFrame:frame]) {
    _drawingView = [PREPPreparedProseDrawingView new];
    _drawingView.backgroundColor = UIColor.clearColor;
    [self addSubview:_drawingView];
  }
  return self;
}

- (void)updateProps:(const Props::Shared &)props oldProps:(const Props::Shared &)oldProps
{
  [super updateProps:props oldProps:oldProps];
  _viewerProps = std::static_pointer_cast<const PreparedProseViewerProps>(props);
  _reportedErrorGeneration = nil;
  [self installMeasuredArtifact];
}

- (void)updateLayoutMetrics:(const LayoutMetrics &)layoutMetrics
            oldLayoutMetrics:(const LayoutMetrics &)oldLayoutMetrics
{
  [super updateLayoutMetrics:layoutMetrics oldLayoutMetrics:oldLayoutMetrics];
  _layoutMetrics = layoutMetrics;
  _drawingView.frame = RCTCGRectFromRect(layoutMetrics.getContentFrame());
  [self installMeasuredArtifact];
}

- (void)prepareForRecycle
{
  [super prepareForRecycle];
  _viewerProps.reset();
  [_drawingView installWithLayout:nil];
  _reportedErrorGeneration = nil;
}

- (void)installMeasuredArtifact
{
  if (!_viewerProps || !std::isfinite(_layoutMetrics.frame.size.width) ||
      _layoutMetrics.frame.size.width <= 0) {
    return;
  }
  const auto &props = *_viewerProps;
  const CGFloat scale = UIScreen.mainScreen.scale > 0 ? UIScreen.mainScreen.scale : 1;
  const BOOL installed = [[PREPPreparedProseLayoutRegistry sharedRegistry]
      installCachedLayoutInDrawingView:_drawingView
                            sourceKind:SourceKind(props)
                                source:StringFromStdString(props.source)
                            configJSON:StringFromStdString(props.configJson)
                             themeJSON:OptionalStringFromStdString(props.themeJson)
                       imagePolicyJSON:OptionalStringFromStdString(props.imagePolicyJson)
                        imagesEnabled:props.imagesEnabled
                  collapsesWhenEmpty:props.collapsesWhenEmpty
                          widthPoints:_layoutMetrics.frame.size.width
                                 scale:scale];
  if (!installed || !_drawingView.errorCode) {
    return;
  }
  const auto generation = [NSString stringWithFormat:@"%@:%@:%@", StringFromStdString(props.source), StringFromStdString(props.configJson), _drawingView.errorCode];
  if ([_reportedErrorGeneration isEqualToString:generation]) {
    return;
  }
  _reportedErrorGeneration = generation;
  const auto eventEmitter = std::static_pointer_cast<const PreparedProseViewerEventEmitter>(_eventEmitter);
  if (eventEmitter) {
    eventEmitter->onError({
        .domain = std::string(_drawingView.errorDomain.UTF8String ?: "viewer"),
        .code = std::string(_drawingView.errorCode.UTF8String),
        .message = std::string(_drawingView.errorMessage.UTF8String ?: "Preparation failed"),
        .fatal = true,
    });
  }
}

@end
