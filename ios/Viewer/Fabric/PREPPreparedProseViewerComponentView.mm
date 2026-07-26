#import "PREPPreparedProseViewerComponentView.h"

#import "ReactNativeProseEditor-Swift.h"

#include <react/renderer/components/PreparedProseViewer/PreparedProseViewerComponentDescriptor.h>
#include <react/renderer/components/PreparedProseViewer/PreparedProseViewerState.h>
#include <react/renderer/components/ReactNativeProseEditorSpec/EventEmitters.h>
#include <react/renderer/components/ReactNativeProseEditorSpec/Props.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>

#include <cmath>
#include <cstring>
#include <limits>
#include <optional>
#include <string>

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

bool HasEquivalentProps(
    const PreparedProseViewerProps &left,
    const PreparedProseViewerProps &right) {
  return left.sourceKind == right.sourceKind && left.source == right.source &&
      left.configJson == right.configJson && left.themeJson == right.themeJson &&
      left.imagePolicyJson == right.imagePolicyJson &&
      left.imagesEnabled == right.imagesEnabled &&
      left.collapsesWhenEmpty == right.collapsesWhenEmpty &&
      left.fontEnvironmentRevision == right.fontEnvironmentRevision;
}

uint64_t Revision(const PreparedProseViewerState *state, bool attachment) {
  if (state == nullptr) {
    return 0;
  }
  return attachment ? state->attachmentRevision : state->nativeFontRevision;
}

std::string GenerationIdentity(
    const PreparedProseViewerProps &props,
    const PreparedProseViewerState *state) {
  return std::string(props.sourceKind == PreparedProseViewerSourceKind::Html ? "html" : "json") +
      "\x1f" + props.source + "\x1f" + props.configJson + "\x1f" +
      (props.themeJson ? *props.themeJson : "") + "\x1f" +
      (props.imagePolicyJson ? *props.imagePolicyJson : "") + "\x1f" +
      (props.imagesEnabled ? "1" : "0") + "\x1f" +
      (props.collapsesWhenEmpty ? "1" : "0") + "\x1f" +
      std::to_string(Revision(state, true)) + "\x1f" +
      std::to_string(Revision(state, false)) + "\x1f" +
      std::to_string(props.fontEnvironmentRevision);
}

uint64_t ScaleBits(CGFloat scale) {
  const double value = scale;
  uint64_t bits = 0;
  static_assert(sizeof(bits) == sizeof(value));
  std::memcpy(&bits, &value, sizeof(bits));
  return bits;
}

std::optional<long long> RoundedWidthPixels(CGFloat width, CGFloat scale) {
  const double physicalWidth = static_cast<double>(width) * static_cast<double>(scale);
  if (!std::isfinite(physicalWidth) || physicalWidth <= 0) {
    return std::nullopt;
  }
  const double roundedWidth = std::round(physicalWidth);
  const double largestConvertible = std::nextafter(
      static_cast<double>(std::numeric_limits<long long>::max()), 0.0);
  if (!std::isfinite(roundedWidth) || roundedWidth <= 0 ||
      roundedWidth > largestConvertible) {
    return std::nullopt;
  }
  return static_cast<long long>(roundedWidth);
}

uint64_t FontEnvironmentRevision(const PreparedProseViewerProps &props) {
  const double value = static_cast<double>(props.fontEnvironmentRevision);
  const double largestConvertible = std::nextafter(
      static_cast<double>(std::numeric_limits<uint64_t>::max()), 0.0);
  return std::isfinite(value) && value > 0 && value <= largestConvertible
      ? static_cast<uint64_t>(value)
      : 0;
}

std::string MeasurementIdentity(
    const PreparedProseViewerProps &props,
    const PreparedProseViewerState *state,
    CGFloat width,
    CGFloat scale) {
  const auto widthPixels = RoundedWidthPixels(width, scale);
  return GenerationIdentity(props, state) + "\x1f" +
      (widthPixels ? std::to_string(*widthPixels) : "invalid") +
      "\x1f" + std::to_string(ScaleBits(scale));
}

bool HasUsableLayoutMetrics(const LayoutMetrics &layoutMetrics) {
  const auto contentFrame = layoutMetrics.getContentFrame();
  return RoundedWidthPixels(contentFrame.size.width, layoutMetrics.pointScaleFactor).has_value();
}

} // namespace

@implementation PREPPreparedProseViewerComponentView {
  PREPPreparedProseDrawingView *_drawingView;
  std::shared_ptr<const PreparedProseViewerProps> _viewerProps;
  std::shared_ptr<const PreparedProseViewerState> _viewerState;
  LayoutMetrics _layoutMetrics;
  BOOL _hasReceivedUsableLayoutMetrics;
  NSString *_reportedErrorGeneration;
  NSString *_installedMeasurementIdentity;
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
  const auto nextProps = std::static_pointer_cast<const PreparedProseViewerProps>(props);
  if (!_viewerProps || !HasEquivalentProps(*_viewerProps, *nextProps)) {
    [self beginNewGeneration];
  }
  _viewerProps = nextProps;
  if (_hasReceivedUsableLayoutMetrics) {
    [self installMeasuredArtifact];
  }
}

- (void)updateState:(const State::Shared &)state oldState:(const State::Shared &)oldState
{
  [super updateState:state oldState:oldState];
  const auto nextState = std::static_pointer_cast<const PreparedProseViewerState>(state);
  if (!_viewerState || Revision(_viewerState.get(), true) != Revision(nextState.get(), true) ||
      Revision(_viewerState.get(), false) != Revision(nextState.get(), false)) {
    [self beginNewGeneration];
  }
  _viewerState = nextState;
  if (_hasReceivedUsableLayoutMetrics) {
    [self installMeasuredArtifact];
  }
}

- (void)updateLayoutMetrics:(const LayoutMetrics &)layoutMetrics
            oldLayoutMetrics:(const LayoutMetrics &)oldLayoutMetrics
{
  [super updateLayoutMetrics:layoutMetrics oldLayoutMetrics:oldLayoutMetrics];
  _layoutMetrics = layoutMetrics;
  _drawingView.frame = RCTCGRectFromRect(layoutMetrics.getContentFrame());
  if (!HasUsableLayoutMetrics(layoutMetrics)) {
    _hasReceivedUsableLayoutMetrics = NO;
    return;
  }
  _hasReceivedUsableLayoutMetrics = YES;
  [self installMeasuredArtifact];
}

- (void)prepareForRecycle
{
  [super prepareForRecycle];
  _viewerProps.reset();
  _viewerState.reset();
  [_drawingView installWithLayout:nil];
  _hasReceivedUsableLayoutMetrics = NO;
  _reportedErrorGeneration = nil;
  _installedMeasurementIdentity = nil;
}

- (void)beginNewGeneration
{
  // Keep the last complete artifact visible while a new generation has no
  // representable layout metrics. `installMeasuredArtifact` clears it only
  // after independently validating the replacement measurement.
  _installedMeasurementIdentity = nil;
  _reportedErrorGeneration = nil;
}

- (void)installMeasuredArtifact
{
  if (!_viewerProps || !HasUsableLayoutMetrics(_layoutMetrics)) {
    return;
  }
  const auto &props = *_viewerProps;
  const auto contentFrame = _layoutMetrics.getContentFrame();
  const CGFloat width = contentFrame.size.width;
  const CGFloat scale = _layoutMetrics.pointScaleFactor;
  const auto measurementIdentity = MeasurementIdentity(props, _viewerState.get(), width, scale);
  const auto measurementIdentityString = StringFromStdString(measurementIdentity);
  if (![_installedMeasurementIdentity isEqualToString:measurementIdentityString]) {
    [_drawingView installWithLayout:nil];
    _installedMeasurementIdentity = nil;
  }
  const BOOL installed = [[PREPPreparedProseLayoutRegistry sharedRegistry]
      installCachedLayoutInDrawingView:_drawingView
                            sourceKind:SourceKind(props)
                                source:StringFromStdString(props.source)
                            configJSON:StringFromStdString(props.configJson)
                             themeJSON:OptionalStringFromStdString(props.themeJson)
                       imagePolicyJSON:OptionalStringFromStdString(props.imagePolicyJson)
                        imagesEnabled:props.imagesEnabled
                  collapsesWhenEmpty:props.collapsesWhenEmpty
                   attachmentRevision:Revision(_viewerState.get(), true)
                   nativeFontRevision:Revision(_viewerState.get(), false)
              fontEnvironmentRevision:FontEnvironmentRevision(props)
                          widthPoints:width
                                 scale:scale];
  if (!installed) {
    return;
  }
  _installedMeasurementIdentity = measurementIdentityString;
  if (!_drawingView.errorCode) {
    return;
  }
  const auto generation = StringFromStdString(GenerationIdentity(props, _viewerState.get()));
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
