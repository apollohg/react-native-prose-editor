require 'json'
package = JSON.parse(File.read(File.join(__dir__, 'package.json')))
Pod::Spec.new do |s|
  s.name = 'NativeEditorCodeHighlighting'
  s.version = package['version']
  s.summary = package['description']
  s.homepage = 'https://github.com/apollohg/react-native-rich-text-editor'
  s.author = 'Apollo HG'
  s.license = { :type => 'Apache-2.0', :file => 'LICENSE' }
  s.source = { :git => 'https://github.com/apollohg/react-native-rich-text-editor.git' }
  s.platforms = { :ios => '16.4' }
  s.swift_version = '5.9'
  s.static_framework = false
  s.dependency 'ExpoModulesCore'
  s.dependency 'ReactNativeProseEditor'
  s.source_files = 'ios/*.swift'
  s.vendored_frameworks = 'ios/NativeEditorHighlighting.xcframework'
  s.preserve_paths = 'ios/native_editor_highlightingFFI/**/*'
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'SWIFT_INCLUDE_PATHS' => '$(PODS_TARGET_SRCROOT)/ios/native_editor_highlightingFFI',
    'HEADER_SEARCH_PATHS' => '$(PODS_TARGET_SRCROOT)/ios/native_editor_highlightingFFI'
  }
end
