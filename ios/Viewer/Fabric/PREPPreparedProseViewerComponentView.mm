#import "PREPPreparedProseViewerComponentView.h"

#import "ReactNativeProseEditor-Swift.h"
#import <React/RCTComponent.h>

#include <react/renderer/components/PreparedProseViewer/PreparedProseViewerComponentDescriptor.h>
#include <react/renderer/components/PreparedProseViewer/PreparedProseViewerShadowNode.h>
#include <react/renderer/components/PreparedProseViewer/PreparedProseViewerState.h>
#include <react/renderer/components/ReactNativeProseEditorSpec/EventEmitters.h>
#include <react/renderer/components/ReactNativeProseEditorSpec/Props.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>

#include <cmath>
#include <cstdint>
#include <cstring>
#include <limits>
#include <memory>
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

bool HasEquivalentGenerationProps(
    const PreparedProseViewerProps &left,
    const PreparedProseViewerProps &right) {
  return left.sourceKind == right.sourceKind && left.source == right.source &&
      left.configJson == right.configJson && left.themeJson == right.themeJson &&
      left.imagePolicyJson == right.imagePolicyJson &&
      left.imagesEnabled == right.imagesEnabled &&
      left.collapsesWhenEmpty == right.collapsesWhenEmpty &&
      left.fontEnvironmentRevision == right.fontEnvironmentRevision;
}

uint64_t Revision(const PreparedProseViewerShadowNode::ConcreteState::Shared &state, bool attachment) {
  if (!state) {
    return 0;
  }
  const auto &data = state->getData();
  return attachment ? data.attachmentRevision : data.nativeFontRevision;
}

uint64_t ScaleBits(CGFloat scale) {
  const double value = scale;
  uint64_t bits = 0;
  static_assert(sizeof(bits) == sizeof(value));
  std::memcpy(&bits, &value, sizeof(bits));
  return bits;
}

std::optional<long long> RoundedWidthPixels(CGFloat width, CGFloat scale) {
  if (!std::isfinite(width) || width <= 0 || !std::isfinite(scale) ||
      scale <= 0) {
    return std::nullopt;
  }
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

NSString *MeasurementIdentity(NSString *generation, CGFloat width, CGFloat scale) {
  const auto widthPixels = RoundedWidthPixels(width, scale);
  return [NSString stringWithFormat:@"%@\x1f%@\x1f%llu",
                                    generation,
                                    widthPixels ? [NSString stringWithFormat:@"%lld", *widthPixels] : @"invalid",
                                    static_cast<unsigned long long>(ScaleBits(scale))];
}

bool HasUsableLayoutMetrics(const LayoutMetrics &layoutMetrics) {
  const auto contentFrame = layoutMetrics.getContentFrame();
  return RoundedWidthPixels(contentFrame.size.width, layoutMetrics.pointScaleFactor).has_value();
}

std::optional<int64_t> SurfaceIdForComponentView(UIView *view) {
  for (UIView *candidate = view; candidate != nil; candidate = candidate.superview) {
    if (RCTIsReactRootView(@(candidate.tag))) {
      return static_cast<int64_t>(candidate.tag);
    }
  }
  return std::nullopt;
}

} // namespace

@interface PREPPreparedProseViewerComponentView () <PreparedProseDrawingViewInteractionDelegate>
@end

@implementation PREPPreparedProseViewerComponentView {
  PREPPreparedProseDrawingView *_drawingView;
  std::shared_ptr<const PreparedProseViewerProps> _viewerProps;
  PreparedProseViewerShadowNode::ConcreteState::Shared _viewerState;
  LayoutMetrics _layoutMetrics;
  BOOL _hasReceivedUsableLayoutMetrics;
  NSString *_reportedErrorGeneration;
  NSString *_installedMeasurementIdentity;
  NSString *_ownedGeneration;
  int64_t _ownedSurfaceId;
  int64_t _ownedComponentTag;
  BOOL _hasOwnedSurface;
  id _imageMetadataObserver;
  id _contentSizeObserver;
}

+ (ComponentDescriptorProvider)componentDescriptorProvider
{
  return concreteComponentDescriptorProvider<PreparedProseViewerComponentDescriptor>();
}

- (instancetype)initWithFrame:(CGRect)frame
{
  if (self = [super initWithFrame:frame]) {
    _drawingView = [PREPPreparedProseDrawingView new];
    _drawingView.interactionDelegate = self;
    _drawingView.backgroundColor = UIColor.clearColor;
    _drawingView.isAccessibilityElement = NO;
    self.isAccessibilityElement = NO;
    [self addSubview:_drawingView];
    __weak PREPPreparedProseViewerComponentView *weakSelf = self;
    _imageMetadataObserver = [[NSNotificationCenter defaultCenter]
        addObserverForName:PREPPreparedProseDrawingView.imageMetadataDidResolve
                    object:_drawingView
                     queue:NSOperationQueue.mainQueue
                usingBlock:^(NSNotification *note) {
                  [weakSelf handleImageMetadata:note];
                }];
    _contentSizeObserver = [[NSNotificationCenter defaultCenter]
        addObserverForName:UIContentSizeCategoryDidChangeNotification
                    object:nil
                     queue:NSOperationQueue.mainQueue
                usingBlock:^(__unused NSNotification *note) {
                  [weakSelf invalidateNativeFontEnvironment];
                }];
  }
  return self;
}

- (void)dealloc
{
  if (_imageMetadataObserver) [[NSNotificationCenter defaultCenter] removeObserver:_imageMetadataObserver];
  if (_contentSizeObserver) [[NSNotificationCenter defaultCenter] removeObserver:_contentSizeObserver];
}

- (BOOL)preparedProseDrawingView:(PREPPreparedProseDrawingView *)view
                 didActivateLink:(NSString *)href
                            text:(NSString *)text
{
  const auto eventEmitter = std::static_pointer_cast<const PreparedProseViewerEventEmitter>(_eventEmitter);
  if (!eventEmitter) return NO;
  eventEmitter->onPressLink({
      .href = std::string(href.UTF8String ?: ""),
      .text = std::string(text.UTF8String ?: ""),
  });
  return YES;
}

- (BOOL)preparedProseDrawingView:(PREPPreparedProseDrawingView *)view
              didActivateMention:(uint32_t)docPos
                            label:(NSString *)label
{
  const auto eventEmitter = std::static_pointer_cast<const PreparedProseViewerEventEmitter>(_eventEmitter);
  if (!eventEmitter) return NO;
  eventEmitter->onPressMention({
      // Codegen's Double contract preserves the complete UInt32 domain in JS.
      .docPos = static_cast<double>(docPos),
      .label = std::string(label.UTF8String ?: ""),
  });
  return YES;
}

- (void)updateProps:(const Props::Shared &)props oldProps:(const Props::Shared &)oldProps
{
  [super updateProps:props oldProps:oldProps];
  const auto nextProps = std::static_pointer_cast<const PreparedProseViewerProps>(props);
  // Link permission filters the installed interaction/accessibility host; it
  // does not alter the prepared render generation or its Fabric lease.
  const BOOL generationChanged =
      !_viewerProps || !HasEquivalentGenerationProps(*_viewerProps, *nextProps);
  if (generationChanged) {
    [self beginNewGeneration];
  }
  _viewerProps = nextProps;
  _drawingView.linkInteractionsEnabled = nextProps->enableLinkTaps;
  if (generationChanged && _hasReceivedUsableLayoutMetrics) {
    [self installMeasuredArtifactIfAttached];
  }
}

- (void)updateState:(const State::Shared &)state oldState:(const State::Shared &)oldState
{
  [super updateState:state oldState:oldState];
  const auto nextState = std::static_pointer_cast<const PreparedProseViewerShadowNode::ConcreteState>(state);
  if (!_viewerState || Revision(_viewerState, true) != Revision(nextState, true) ||
      Revision(_viewerState, false) != Revision(nextState, false)) {
    [self beginNewGeneration];
  }
  _viewerState = nextState;
  if (_hasReceivedUsableLayoutMetrics) {
    [self installMeasuredArtifactIfAttached];
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
  [self installMeasuredArtifactIfAttached];
}

- (void)didMoveToSuperview
{
  [super didMoveToSuperview];
  if (self.superview == nil) {
    // Detachment may precede recycling and React Native may reset `tag`
    // before prepareForRecycle. Drop only the persisted generation token.
    [self releaseFabricOwnership];
    return;
  }
  // Fabric can provide props, state, and layout metrics before the view is
  // attached to its React root. The update callbacks intentionally defer in
  // that state; attachment is the first point where the surface token is
  // resolvable. The shared seam is idempotent, so this cannot duplicate an
  // installation or error event after a normal layout-driven mount.
  [self installMeasuredArtifactIfAttached];
}

- (void)prepareForRecycle
{
  [self releaseAllFabricOwnership];
  [super prepareForRecycle];
  _viewerProps.reset();
  _viewerState.reset();
  [_drawingView cancelConfiguredImages];
  [_drawingView installWithLayout:nil];
  _hasReceivedUsableLayoutMetrics = NO;
  _reportedErrorGeneration = nil;
  _installedMeasurementIdentity = nil;
  _ownedGeneration = nil;
}

- (void)beginNewGeneration
{
  [self releaseFabricOwnership];
  [_drawingView cancelConfiguredImages];
  // Keep the last complete artifact visible while a new generation has no
  // representable layout metrics. The install gate clears it only
  // after independently validating the replacement measurement.
  _installedMeasurementIdentity = nil;
  _reportedErrorGeneration = nil;
}

- (void)releaseFabricOwnership
{
  if (!_hasOwnedSurface || !_ownedGeneration) {
    return;
  }
  [[PREPPreparedProseLayoutRegistry sharedRegistry]
      releaseFabricGenerationSurfaceId:_ownedSurfaceId
                          componentTag:_ownedComponentTag
                    generationIdentity:_ownedGeneration];
  _hasOwnedSurface = NO;
  _ownedGeneration = nil;
}

- (void)releaseAllFabricOwnership
{
  // Recycling must use the same persisted canonical key as replacement,
  // detachment, and mount-miss cleanup. Surface-wide release could otherwise
  // remove a generation the recycled view never owned.
  [self releaseFabricOwnership];
}

- (void)installMeasuredArtifactIfAttached
{
  // This is a deliberate lifecycle test seam: updateProps, updateState,
  // updateLayoutMetrics, and didMoveToSuperview all enter through this gate.
  // It must never acquire or dispatch while the component is detached.
  if (self.superview == nil || !_viewerProps || !_viewerState ||
      !_hasReceivedUsableLayoutMetrics || !HasUsableLayoutMetrics(_layoutMetrics)) {
    return;
  }
  const auto &props = *_viewerProps;
  const auto contentFrame = _layoutMetrics.getContentFrame();
  const CGFloat width = contentFrame.size.width;
  const CGFloat scale = _layoutMetrics.pointScaleFactor;
  const auto surfaceId = SurfaceIdForComponentView(self);
  if (!surfaceId) {
    return;
  }
  const auto componentTag = static_cast<int64_t>(self.tag);
  const auto generation = [[PREPPreparedProseLayoutRegistry sharedRegistry]
      fabricGenerationIdentitySourceKind:SourceKind(props)
                              source:StringFromStdString(props.source)
                          configJSON:StringFromStdString(props.configJson)
                           themeJSON:OptionalStringFromStdString(props.themeJson)
                     imagePolicyJSON:OptionalStringFromStdString(props.imagePolicyJson)
                      imagesEnabled:props.imagesEnabled
                collapsesWhenEmpty:props.collapsesWhenEmpty
                 attachmentRevision:Revision(_viewerState, true)
                 nativeFontRevision:Revision(_viewerState, false)
            fontEnvironmentRevision:FontEnvironmentRevision(props)];
  const auto measurementIdentityString = MeasurementIdentity(generation, width, scale);
  if (_hasOwnedSurface && _ownedSurfaceId == *surfaceId &&
      _ownedComponentTag == componentTag &&
      [_ownedGeneration isEqualToString:generation] &&
      [_installedMeasurementIdentity isEqualToString:measurementIdentityString]) {
    return;
  }
  if (_hasOwnedSurface &&
      (_ownedSurfaceId != *surfaceId || _ownedComponentTag != componentTag ||
       ![_ownedGeneration isEqualToString:generation])) {
    // A reused view may have a different root/token before the previous
    // lifecycle callback finishes. Release only the persisted owner, never
    // the current UIView tag, which React Native may already have reset.
    [self releaseFabricOwnership];
  }
  if (![_installedMeasurementIdentity isEqualToString:measurementIdentityString]) {
    [_drawingView installWithLayout:nil];
    _installedMeasurementIdentity = nil;
  }

  // Establish ownership before acquisition. A mount miss otherwise leaves a
  // measurement-pinned compiler result or failure alive with no installed
  // component to release it.
  _ownedSurfaceId = *surfaceId;
  _ownedComponentTag = componentTag;
  _hasOwnedSurface = YES;
  _ownedGeneration = generation;
  const BOOL installed = [[PREPPreparedProseLayoutRegistry sharedRegistry]
      installCachedLayoutInDrawingView:_drawingView
                             surfaceId:_ownedSurfaceId
                          componentTag:_ownedComponentTag
                            sourceKind:SourceKind(props)
                                source:StringFromStdString(props.source)
                            configJSON:StringFromStdString(props.configJson)
                             themeJSON:OptionalStringFromStdString(props.themeJson)
                       imagePolicyJSON:OptionalStringFromStdString(props.imagePolicyJson)
                        imagesEnabled:props.imagesEnabled
                  collapsesWhenEmpty:props.collapsesWhenEmpty
                   attachmentRevision:Revision(_viewerState, true)
                   nativeFontRevision:Revision(_viewerState, false)
              fontEnvironmentRevision:FontEnvironmentRevision(props)
                          widthPoints:width
                                 scale:scale];
  if (!installed) {
    // The lease and generation pin were created by Yoga, but Fabric found no
    // artifact to install. Release this exact persisted owner immediately.
    [[PREPPreparedProseLayoutRegistry sharedRegistry]
        releaseFabricMountMissSurfaceId:_ownedSurfaceId
                           componentTag:_ownedComponentTag
                     generationIdentity:_ownedGeneration];
    _hasOwnedSurface = NO;
    _ownedGeneration = nil;
    return;
  }
  [_drawingView configureImagesWithGeneration:generation
                                imagesEnabled:props.imagesEnabled
                                  policyJSON:OptionalStringFromStdString(props.imagePolicyJson)];
  _installedMeasurementIdentity = measurementIdentityString;
  if (!_drawingView.errorCode) {
    return;
  }
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

- (void)handleImageMetadata:(NSNotification *)note
{
  NSString *generation = note.userInfo[@"generation"];
  if (!generation || !_viewerState || ![_ownedGeneration isEqualToString:generation]) return;
  _viewerState->updateState(
      [](const PreparedProseViewerShadowNode::ConcreteState::Data &oldData)
          -> PreparedProseViewerShadowNode::ConcreteState::SharedData {
        auto nextData = oldData;
        nextData.attachmentRevision += 1;
        return std::make_shared<const PreparedProseViewerShadowNode::ConcreteState::Data>(nextData);
      });
}

- (void)invalidateNativeFontEnvironment
{
  if (!_viewerState) return;
  _viewerState->updateState(
      [](const PreparedProseViewerShadowNode::ConcreteState::Data &oldData)
          -> PreparedProseViewerShadowNode::ConcreteState::SharedData {
        auto nextData = oldData;
        nextData.nativeFontRevision += 1;
        return std::make_shared<const PreparedProseViewerShadowNode::ConcreteState::Data>(nextData);
      });
}

@end
