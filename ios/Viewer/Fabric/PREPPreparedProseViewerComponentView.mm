#import "PREPPreparedProseViewerComponentView.h"

#if __has_include("ReactNativeProseEditor-Swift.h")
#import "ReactNativeProseEditor-Swift.h"
#else
#error "ReactNativeProseEditor Swift compatibility header is unavailable; verify the pod module name and consumer codegen"
#endif
#import <React/RCTConversions.h>

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

// Attachment publication belongs to the immutable source/configuration, not
// to Fabric's derived attachment/font revisions or a layout-only replacement.
uint64_t Revision(const PreparedProseViewerShadowNode::ConcreteState::Shared &state, bool attachment) {
  if (!state) {
    return 0;
  }
  const auto &data = state->getData();
  return attachment ? data.attachmentRevision : data.nativeFontRevision;
}

CGFloat NativeFontScale(const PreparedProseViewerShadowNode::ConcreteState::Shared &state) {
  if (!state) return 1;
  const auto value = state->getData().nativeFontScale;
  return std::isfinite(value) && value > 0 ? static_cast<CGFloat>(value) : 1;
}

int32_t UserInterfaceStyle(const PreparedProseViewerShadowNode::ConcreteState::Shared &state) {
  if (!state) return 0;
  return state->getData().userInterfaceStyle;
}

int32_t EffectiveUserInterfaceStyle(UIView *view) {
  return view.traitCollection.userInterfaceStyle == UIUserInterfaceStyleDark ? 2 : 1;
}

uint64_t LeaseHandle(const PreparedProseViewerShadowNode::ConcreteState::Shared &state) {
  return state ? state->getData().leaseHandle : 0;
}

void DeactivateLease(
    const PreparedProseViewerShadowNode::ConcreteState::Shared &state,
    uint64_t leaseHandle) {
  if (!state || state->getData().leaseHandle != leaseHandle) return;
  const auto &lifecycle = state->getData().leaseLifecycle;
  if (lifecycle) lifecycle->deactivate();
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

NSString *MeasurementIdentity(NSString *generation, CGFloat width, CGFloat scale, uint64_t leaseHandle) {
  const auto widthPixels = RoundedWidthPixels(width, scale);
  return [NSString stringWithFormat:@"%@\x1f%@\x1f%llu\x1f%llu",
                                    generation,
                                    widthPixels ? [NSString stringWithFormat:@"%lld", *widthPixels] : @"invalid",
                                    static_cast<unsigned long long>(ScaleBits(scale)),
                                    static_cast<unsigned long long>(leaseHandle)];
}

bool HasUsableLayoutMetrics(const LayoutMetrics &layoutMetrics) {
  const auto contentFrame = layoutMetrics.getContentFrame();
  return RoundedWidthPixels(contentFrame.size.width, layoutMetrics.pointScaleFactor).has_value();
}

int64_t SurfaceIdFromState(
    const PreparedProseViewerShadowNode::ConcreteState::Shared &state) {
  return state ? static_cast<int64_t>(state->getData().surfaceId) : 0;
}

int64_t ComponentTagFromState(
    const PreparedProseViewerShadowNode::ConcreteState::Shared &state) {
  return state ? static_cast<int64_t>(state->getData().componentTag) : 0;
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
  NSString *_ownedSemanticGeneration;
  int64_t _ownedSurfaceId;
  int64_t _ownedComponentTag;
  uint64_t _ownedLeaseHandle;
  BOOL _hasOwnedSurface;
  id _imageMetadataObserver;
  id _imageResourceObserver;
  id _fontEnvironmentObserver;
  uint64_t _lastFontEnvironmentRevision;
  CGFloat _fontEnvironmentScale;
  int32_t _requestedUserInterfaceStyle;
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
    _imageResourceObserver = [[NSNotificationCenter defaultCenter]
        addObserverForName:PREPPreparedProseDrawingView.imageResourceDidFail
                    object:_drawingView
                     queue:NSOperationQueue.mainQueue
                usingBlock:^(__unused NSNotification *note) {
                  [weakSelf handleImageResourceFailure:note];
                }];
    PREPViewerFontEnvironment *fontEnvironment = [PREPViewerFontEnvironment sharedEnvironment];
    [fontEnvironment refreshContentSizeCategory];
    _fontEnvironmentScale = [fontEnvironment currentFontScale];
    _requestedUserInterfaceStyle = 0;
    _fontEnvironmentObserver = [[NSNotificationCenter defaultCenter]
        addObserverForName:PREPViewerFontEnvironment.didInvalidateNotification
                    object:fontEnvironment
                     queue:NSOperationQueue.mainQueue
                usingBlock:^(NSNotification *note) {
                  [weakSelf handleFontEnvironmentInvalidation:note];
                }];
  }
  return self;
}

- (void)dealloc
{
  // Deallocation can follow a Fabric discard without prepareForRecycle. This
  // helper is idempotent and talks only to the stable registry token; it never
  // asks a detached UIView for its current tag or root surface.
  [self releaseAllFabricOwnership];
  if (_imageMetadataObserver) [[NSNotificationCenter defaultCenter] removeObserver:_imageMetadataObserver];
  if (_imageResourceObserver) [[NSNotificationCenter defaultCenter] removeObserver:_imageResourceObserver];
  if (_fontEnvironmentObserver) [[NSNotificationCenter defaultCenter] removeObserver:_fontEnvironmentObserver];
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
                        attrsJSON:(NSString *)attrsJSON
{
  const auto eventEmitter = std::static_pointer_cast<const PreparedProseViewerEventEmitter>(_eventEmitter);
  if (!eventEmitter) return NO;
  eventEmitter->onPressMention({
      // Codegen's Double contract preserves the complete UInt32 domain in JS.
      .docPos = static_cast<double>(docPos),
      .label = std::string(label.UTF8String ?: ""),
      .attrsJson = std::string(attrsJSON.UTF8String ?: ""),
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
    [self beginNewGenerationTerminatingCurrentLease:NO];
  }
  _viewerProps = nextProps;
  [self beginSemanticImageGenerationIfPossible];
  _drawingView.linkInteractionsEnabled = nextProps->enableLinkTaps;
  if (generationChanged && _hasReceivedUsableLayoutMetrics) {
    [self installMeasuredArtifactIfAttached];
  }
}

- (void)updateState:(const State::Shared &)state oldState:(const State::Shared &)oldState
{
  [super updateState:state oldState:oldState];
  const auto nextState = std::static_pointer_cast<const PreparedProseViewerShadowNode::ConcreteState>(state);
  const BOOL leaseChanged = _viewerState && LeaseHandle(_viewerState) != LeaseHandle(nextState);
  if (!_viewerState || Revision(_viewerState, true) != Revision(nextState, true) ||
      Revision(_viewerState, false) != Revision(nextState, false) ||
      NativeFontScale(_viewerState) != NativeFontScale(nextState) ||
      leaseChanged) {
    // Revisions and same-family width updates are replacements, not a
    // teardown.  Only a genuinely different state incarnation retires the
    // previous lifecycle guard.
    [self beginNewGenerationTerminatingCurrentLease:leaseChanged];
  }
  _viewerState = nextState;
  _requestedUserInterfaceStyle = UserInterfaceStyle(_viewerState);
  const auto userInterfaceStyle = EffectiveUserInterfaceStyle(self);
  if (UserInterfaceStyle(_viewerState) != userInterfaceStyle) {
    [self invalidateUserInterfaceStyle:userInterfaceStyle];
    return;
  }
  if (NativeFontScale(_viewerState) != _fontEnvironmentScale) {
    [self invalidateNativeFontEnvironmentWithScale:_fontEnvironmentScale];
  }
  [self beginSemanticImageGenerationIfPossible];
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
  [_drawingView updateConfiguredImagesForVisibleWindow];
}

- (void)didMoveToSuperview
{
  [super didMoveToSuperview];
  if (self.superview == nil) {
    // Detachment is a normal Fabric transaction boundary. The state-owned
    // lifetime guard performs terminal cleanup if this family is discarded;
    // retaining a same-family lease here lets an ordinary reattachment mount
    // the already prepared artifact without reminting a handle.
    [_drawingView cancelConfiguredImages];
    return;
  }
  // Fabric can provide props, state, and metrics before native attachment.
  [self installMeasuredArtifactIfAttached];
  [_drawingView updateConfiguredImagesForVisibleWindow];
}

- (void)didMoveToWindow
{
  [super didMoveToWindow];
  if (self.window == nil) {
    return;
  }
  [self installMeasuredArtifactIfAttached];
  [_drawingView updateConfiguredImagesForVisibleWindow];
}

- (void)traitCollectionDidChange:(UITraitCollection *)previousTraitCollection
{
  [super traitCollectionDidChange:previousTraitCollection];
  [self invalidateUserInterfaceStyle:EffectiveUserInterfaceStyle(self)];
}

- (void)prepareForRecycle
{
  [self releaseAllFabricOwnership];
  [super prepareForRecycle];
  _viewerProps.reset();
  _viewerState.reset();
  [_drawingView cancelConfiguredImages];
  [_drawingView resetIntrinsicImagePublication];
  [_drawingView installWithLayout:nil];
  _hasReceivedUsableLayoutMetrics = NO;
  _reportedErrorGeneration = nil;
  _installedMeasurementIdentity = nil;
  _ownedGeneration = nil;
  _ownedSemanticGeneration = nil;
  _ownedLeaseHandle = 0;
}

- (void)beginNewGenerationTerminatingCurrentLease:(BOOL)terminal
{
  // Ordinary props/state revisions are committed by
  // installMeasuredArtifactIfAttached, where the registry can atomically
  // permit G2 before retiring G1's pending work. Releasing here would let a
  // delayed G1 Yoga callback win that race and would blank the old mount.
  if (terminal) {
    [self releaseFabricOwnershipTerminatingLease:YES];
  }
  [_drawingView cancelConfiguredImages];
  // Keep the last complete artifact visible while a new generation has no
  // representable layout metrics. The install gate clears it only
  // after independently validating the replacement measurement.
  _installedMeasurementIdentity = nil;
}

- (void)beginSemanticImageGenerationIfPossible
{
  if (!_viewerProps || !_viewerState) return;
  const auto &props = *_viewerProps;
  const auto semanticGeneration = [[PREPPreparedProseLayoutRegistry sharedRegistry]
      fabricSemanticGenerationIdentitySourceKind:SourceKind(props)
                              source:StringFromStdString(props.source)
                          configJSON:StringFromStdString(props.configJson)
                           themeJSON:OptionalStringFromStdString(props.themeJson)
                     imagePolicyJSON:OptionalStringFromStdString(props.imagePolicyJson)
                      imagesEnabled:props.imagesEnabled
                collapsesWhenEmpty:props.collapsesWhenEmpty
                 attachmentRevision:Revision(_viewerState, true)
                 nativeFontRevision:Revision(_viewerState, false)
                   nativeFontScale:NativeFontScale(_viewerState)
            fontEnvironmentRevision:FontEnvironmentRevision(props)
               userInterfaceStyle:UserInterfaceStyle(_viewerState)];
  [_drawingView beginSemanticImageGeneration:semanticGeneration];
}

- (void)releaseFabricOwnershipTerminatingLease:(BOOL)terminal
{
  // A state handle represents the Fabric family lifetime, not one props,
  // image, font, or width revision.  Only recycle/family teardown may
  // deactivate it; ordinary replacement releases its old generation key.
  const auto stateLeaseHandle = LeaseHandle(_viewerState);
  if (terminal && stateLeaseHandle != 0) {
    DeactivateLease(_viewerState, stateLeaseHandle);
  }
  if (!_hasOwnedSurface) {
    return;
  }
  const auto leaseHandle = _ownedLeaseHandle != 0 ? _ownedLeaseHandle : stateLeaseHandle;
  if (terminal) {
    // A replacement can retain a mounted G1 while G2 is the only current
    // generation.  Terminal recycle must sweep the state-family owner, not
    // merely G2; this API intentionally works after the handle is inactive.
    // The C++ lifecycle guard may subsequently repeat this exact cleanup.
    [[PREPPreparedProseLayoutRegistry sharedRegistry]
        releaseFabricLeaseSurfaceId:_ownedSurfaceId
                        componentTag:_ownedComponentTag
                        leaseHandle:leaseHandle];
    _hasOwnedSurface = NO;
    _ownedGeneration = nil;
    _ownedLeaseHandle = 0;
    return;
  }
  if (!_ownedGeneration) {
    return;
  }
  [[PREPPreparedProseLayoutRegistry sharedRegistry]
      releaseFabricGenerationSurfaceId:_ownedSurfaceId
                          componentTag:_ownedComponentTag
                    generationIdentity:_ownedGeneration
                        leaseHandle:leaseHandle];
  _hasOwnedSurface = NO;
  _ownedGeneration = nil;
  _ownedLeaseHandle = 0;
}

- (void)releaseAllFabricOwnership
{
  // Recycling is terminal for this exact state-carried lease incarnation.
  [self releaseFabricOwnershipTerminatingLease:YES];
}

- (void)installMeasuredArtifactIfAttached
{
  // This is a deliberate lifecycle test seam: updateProps, updateState,
  // updateLayoutMetrics, didMoveToSuperview, and didMoveToWindow all enter
  // through this gate.
  // It must never acquire or dispatch while the component is detached.
  if (self.superview == nil || !_viewerProps || !_viewerState ||
      !_hasReceivedUsableLayoutMetrics || !HasUsableLayoutMetrics(_layoutMetrics)) {
    return;
  }
  const auto &props = *_viewerProps;
  const auto contentFrame = _layoutMetrics.getContentFrame();
  const CGFloat width = contentFrame.size.width;
  const CGFloat scale = _layoutMetrics.pointScaleFactor;
  const auto surfaceId = SurfaceIdFromState(_viewerState);
  const auto componentTag = ComponentTagFromState(_viewerState);
  if (surfaceId <= 0 || componentTag <= 0) {
    return;
  }
  const auto leaseHandle = LeaseHandle(_viewerState);
  // State is the Fabric handoff. Until the shadow node has committed its
  // opaque handle this view has no authority to acquire or release anything.
  if (leaseHandle == 0) {
    return;
  }
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
                   nativeFontScale:NativeFontScale(_viewerState)
            fontEnvironmentRevision:FontEnvironmentRevision(props)
               userInterfaceStyle:UserInterfaceStyle(_viewerState)];
  const auto measurementIdentityString = MeasurementIdentity(generation, width, scale, leaseHandle);
  const auto semanticGeneration = [[PREPPreparedProseLayoutRegistry sharedRegistry]
      fabricSemanticGenerationIdentitySourceKind:SourceKind(props)
                              source:StringFromStdString(props.source)
                          configJSON:StringFromStdString(props.configJson)
                           themeJSON:OptionalStringFromStdString(props.themeJson)
                     imagePolicyJSON:OptionalStringFromStdString(props.imagePolicyJson)
                      imagesEnabled:props.imagesEnabled
                collapsesWhenEmpty:props.collapsesWhenEmpty
                 attachmentRevision:Revision(_viewerState, true)
                 nativeFontRevision:Revision(_viewerState, false)
                   nativeFontScale:NativeFontScale(_viewerState)
               fontEnvironmentRevision:FontEnvironmentRevision(props)
                  userInterfaceStyle:UserInterfaceStyle(_viewerState)];
  // This is the props/state commit boundary for the state-family handle.
  // Commit G2 before touching G1 ownership or attempting mount acquisition:
  // delayed G1 Yoga callbacks are rejected, while an already-running G2
  // measurement remains permitted to publish its exact handoff.
  [[PREPPreparedProseLayoutRegistry sharedRegistry]
      activateFabricGenerationSurfaceId:surfaceId
                            componentTag:componentTag
                      generationIdentity:generation
                          leaseHandle:leaseHandle];
  if (_hasOwnedSurface && _ownedSurfaceId == surfaceId &&
      _ownedComponentTag == componentTag &&
      [_ownedGeneration isEqualToString:generation] &&
      _ownedLeaseHandle == leaseHandle &&
      [_installedMeasurementIdentity isEqualToString:measurementIdentityString]) {
    return;
  }
  if (_hasOwnedSurface &&
      (_ownedSurfaceId != surfaceId || _ownedComponentTag != componentTag ||
       _ownedLeaseHandle != leaseHandle)) {
    // A reused view may have a different root/token before the previous
    // lifecycle callback finishes. Release only the persisted owner, never
    // the current UIView tag, which React Native may already have reset.
    [self releaseFabricOwnershipTerminatingLease:NO];
  }
  const BOOL preservesMountedArtifactForReplacement =
            _hasOwnedSurface && _ownedSurfaceId == surfaceId &&
      _ownedComponentTag == componentTag &&
      _ownedLeaseHandle == leaseHandle &&
      _installedMeasurementIdentity != nil;
  if (![_installedMeasurementIdentity isEqualToString:measurementIdentityString]) {
    // Width-only replacement is two-phase: retain the currently installed
    // artifact until the exact new Yoga handoff is acquired. A missing or
    // pressure-evicted replacement must not blank the view or release the
    // old mounted lease. Semantic/recycled-owner changes still clear first.
    if (!preservesMountedArtifactForReplacement) {
      [_drawingView installWithLayout:nil];
      _installedMeasurementIdentity = nil;
    }
  }

  // Establish ownership before acquisition. A mount miss otherwise leaves a
  // measurement-pinned compiler result or failure alive with no installed
  // component to release it.
  _ownedSurfaceId = surfaceId;
  _ownedComponentTag = componentTag;
  _ownedLeaseHandle = leaseHandle;
  _hasOwnedSurface = YES;
  _ownedGeneration = generation;
  [_drawingView bindFabricAttachmentStateSurfaceId:_ownedSurfaceId
                                      componentTag:_ownedComponentTag
                                      leaseHandle:leaseHandle];
  const BOOL installed = [[PREPPreparedProseLayoutRegistry sharedRegistry]
      installCachedLayoutInDrawingView:_drawingView
                             surfaceId:_ownedSurfaceId
                          componentTag:_ownedComponentTag
                           leaseHandle:leaseHandle
                            sourceKind:SourceKind(props)
                                source:StringFromStdString(props.source)
                            configJSON:StringFromStdString(props.configJson)
                             themeJSON:OptionalStringFromStdString(props.themeJson)
                       imagePolicyJSON:OptionalStringFromStdString(props.imagePolicyJson)
                        imagesEnabled:props.imagesEnabled
                  collapsesWhenEmpty:props.collapsesWhenEmpty
                   attachmentRevision:Revision(_viewerState, true)
                   nativeFontRevision:Revision(_viewerState, false)
                     nativeFontScale:NativeFontScale(_viewerState)
             fontEnvironmentRevision:FontEnvironmentRevision(props)
                userInterfaceStyle:UserInterfaceStyle(_viewerState)
                          widthPoints:width
                                 scale:scale];
  if (!installed) {
    // The lease and generation pin were created by Yoga, but Fabric found no
    // artifact to install. A width replacement may have an older mounted
    // artifact, which remains valid until a replacement successfully mounts.
    if (preservesMountedArtifactForReplacement) {
      return;
    }
    [[PREPPreparedProseLayoutRegistry sharedRegistry]
        releaseFabricMountMissSurfaceId:_ownedSurfaceId
                           componentTag:_ownedComponentTag
                     generationIdentity:_ownedGeneration
                           leaseHandle:_ownedLeaseHandle
                           widthPoints:width
                                  scale:scale];
    _hasOwnedSurface = NO;
    _ownedGeneration = nil;
    _ownedLeaseHandle = 0;
    return;
  }
  _ownedSemanticGeneration = semanticGeneration;
  [_drawingView configureImagesWithGeneration:semanticGeneration
                                imagesEnabled:props.imagesEnabled
                                  policyJSON:OptionalStringFromStdString(props.imagePolicyJson)];
  [_drawingView updateConfiguredImagesForVisibleWindow];
  _installedMeasurementIdentity = measurementIdentityString;
  if (!_drawingView.errorCode) {
    return;
  }
  if ([_reportedErrorGeneration isEqualToString:semanticGeneration]) {
    return;
  }
  _reportedErrorGeneration = semanticGeneration;
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
  if (!generation || !_viewerState || ![_ownedSemanticGeneration isEqualToString:generation]) return;
  _viewerState->updateState(
      [](const PreparedProseViewerShadowNode::ConcreteState::Data &oldData)
          -> PreparedProseViewerShadowNode::ConcreteState::SharedData {
        auto nextData = oldData;
        nextData.attachmentRevision += 1;
        return std::make_shared<const PreparedProseViewerShadowNode::ConcreteState::Data>(nextData);
      });
}

- (void)invalidateNativeFontEnvironmentWithScale:(CGFloat)scale
{
  if (!_viewerState) return;
  _viewerState->updateState(
      [scale](const PreparedProseViewerShadowNode::ConcreteState::Data &oldData)
          -> PreparedProseViewerShadowNode::ConcreteState::SharedData {
        auto nextData = oldData;
        nextData.nativeFontRevision += 1;
        nextData.nativeFontScale = std::isfinite(scale) && scale > 0 ? scale : 1;
        return std::make_shared<const PreparedProseViewerShadowNode::ConcreteState::Data>(nextData);
      });
}

- (void)invalidateUserInterfaceStyle:(int32_t)userInterfaceStyle
{
  if (!_viewerState || _requestedUserInterfaceStyle == userInterfaceStyle) return;
  _requestedUserInterfaceStyle = userInterfaceStyle;
  _viewerState->updateState(
      [userInterfaceStyle](const PreparedProseViewerShadowNode::ConcreteState::Data &oldData)
          -> PreparedProseViewerShadowNode::ConcreteState::SharedData {
        auto nextData = oldData;
        nextData.nativeFontRevision += 1;
        nextData.userInterfaceStyle = userInterfaceStyle;
        return std::make_shared<const PreparedProseViewerShadowNode::ConcreteState::Data>(nextData);
      });
}

- (void)handleFontEnvironmentInvalidation:(NSNotification *)note
{
  NSNumber *revision = note.userInfo[@"revision"];
  if (!revision || revision.unsignedLongLongValue <= _lastFontEnvironmentRevision) return;
  _lastFontEnvironmentRevision = revision.unsignedLongLongValue;
  NSNumber *scale = note.userInfo[@"scale"];
  _fontEnvironmentScale = std::isfinite(scale.doubleValue) && scale.doubleValue > 0
      ? scale.doubleValue
      : 1;
  [self invalidateNativeFontEnvironmentWithScale:_fontEnvironmentScale];
}

- (void)handleImageResourceFailure:(NSNotification *)note
{
  NSString *generation = note.userInfo[@"generation"];
  if (!generation || ![generation isEqualToString:_ownedSemanticGeneration]) return;
  const auto eventEmitter = std::static_pointer_cast<const PreparedProseViewerEventEmitter>(_eventEmitter);
  if (!eventEmitter) return;
  eventEmitter->onError({
      .domain = "viewer.resource",
      .code = "RESOURCE_LOAD_FAILED",
      .message = "An image resource could not be loaded.",
      .fatal = false,
  });
}

@end
