require 'json'

package = JSON.parse(File.read(File.join(__dir__, '..', 'package.json')))

Pod::Spec.new do |s|
  s.name           = 'ReactNativeProseEditor'
  s.version        = package['version']
  s.summary        = package['description']
  s.description    = package['description']
  s.license        = { :type => 'MIT' }
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

  # Swift source files (including generated UniFFI bindings)
  s.source_files = '**/*.{h,m,swift}'
  s.exclude_files = 'Tests/**/*'

  # Prebuilt Rust static library as XCFramework. CocoaPods only reliably
  # picks up vendored binaries that live under the pod root, so build-ios.sh
  # syncs the generated XCFramework into this ios/ directory.
  xcframework_path = File.join(__dir__, 'EditorCore.xcframework')

  if File.exist?(xcframework_path)
    s.vendored_frameworks = 'EditorCore.xcframework'
  end

  # The UniFFI C header and modulemap for the Rust FFI layer
  s.preserve_paths = [
    'editor_coreFFI/**/*',
    'EditorCore.xcframework/**/*',
  ]

  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'SWIFT_COMPILATION_MODE' => 'wholemodule',
    'SWIFT_INCLUDE_PATHS' => '$(PODS_TARGET_SRCROOT)/editor_coreFFI',
    'HEADER_SEARCH_PATHS' => '$(PODS_TARGET_SRCROOT)/editor_coreFFI',
  }
end
