require 'json'

package = JSON.parse(File.read(File.join(__dir__, 'package.json')))

unless ENV['RCT_NEW_ARCH_ENABLED'] == '1'
  raise 'ReactNativeProseEditor requires the React Native New Architecture. Set RCT_NEW_ARCH_ENABLED=1.'
end

Pod::Spec.new do |s|
  s.name           = 'ReactNativeProseEditor'
  s.version        = package['version']
  s.summary        = package['description']
  s.description    = package['description']
  s.license        = { :type => 'Apache-2.0', :file => 'LICENSE' }
  s.author         = 'Apollo HG'
  s.homepage       = 'https://github.com/apollohg/react-native-prose-editor'
  s.platforms      = { :ios => '15.1' }
  s.swift_version  = '5.9'
  s.source         = { git: 'https://github.com/apollohg/react-native-prose-editor.git' }
  # UniFFI's generated Swift bindings import a companion Clang module
  # (`editor_coreFFI`) via a custom modulemap. CocoaPods does not support
  # custom module maps on Swift static libraries, so this pod must build as
  # a framework.
  s.static_framework = false

  s.dependency 'ExpoModulesCore'

  # Swift source files (including generated UniFFI bindings).
  s.source_files = ['ios/*.swift', 'ios/Viewer/**/*.{swift,h,mm}', 'common/cpp/**/*.{h,cpp}']
  # These Objective-C++/C++ Fabric implementation headers import React C++
  # headers. Keeping them private prevents Swift's umbrella module from
  # importing them while preserving their availability to this pod's sources.
  s.private_header_files = [
    'ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.h',
    'common/cpp/react/renderer/components/PreparedProseViewer/**/*.h',
  ]
  s.header_dir = 'react/renderer/components/PreparedProseViewer'

  # Prebuilt Rust static library as XCFramework.
  s.vendored_frameworks = 'ios/EditorCore.xcframework'

  # The UniFFI C header and modulemap for the Rust FFI layer.
  s.preserve_paths = [
    'ios/editor_coreFFI/**/*',
    'ios/EditorCore.xcframework/**/*',
  ]

  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'SWIFT_COMPILATION_MODE' => 'wholemodule',
    'SWIFT_INCLUDE_PATHS' => '$(PODS_TARGET_SRCROOT)/ios/editor_coreFFI',
    'HEADER_SEARCH_PATHS' => '$(PODS_TARGET_SRCROOT)/ios/editor_coreFFI $(PODS_TARGET_SRCROOT)/common/cpp',
  }

  # React Native must merge its Fabric and Yoga dependency settings after this
  # package-owned configuration. In particular, do not replace the helper's
  # generated private Yoga search path with a hand-maintained equivalent.
  install_modules_dependencies(s)
end
