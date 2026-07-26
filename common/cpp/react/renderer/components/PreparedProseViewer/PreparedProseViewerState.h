#pragma once

#include <cmath>
#include <cstdint>

#ifdef ANDROID
#include <folly/dynamic.h>
#endif

namespace facebook::react {

struct PreparedProseViewerState final {
  uint64_t attachmentRevision{0};
  uint64_t nativeFontRevision{0};
  // A native Dynamic Type invalidation carries the scale that caused it.
  // The JS revision can remain unchanged, so it cannot select this snapshot.
  double nativeFontScale{1.0};

#ifdef ANDROID
  PreparedProseViewerState() = default;

  PreparedProseViewerState(
      const PreparedProseViewerState& /*previousState*/,
      const folly::dynamic& data)
      : attachmentRevision(revisionValue(data, "attachmentRevision")),
        nativeFontRevision(revisionValue(data, "nativeFontRevision")),
        nativeFontScale(scaleValue(data, "nativeFontScale")) {}

  folly::dynamic getDynamic() const {
    return folly::dynamic::object("attachmentRevision", attachmentRevision)(
        "nativeFontRevision", nativeFontRevision)(
        "nativeFontScale", nativeFontScale);
  }

 private:
  static uint64_t revisionValue(const folly::dynamic& data, const char* key) {
    const auto value = data.getDefault(key, 0).asInt();
    return value < 0 ? 0 : static_cast<uint64_t>(value);
  }

  static double scaleValue(const folly::dynamic& data, const char* key) {
    const auto value = data.getDefault(key, 1.0).asDouble();
    return std::isfinite(value) && value > 0.0 ? value : 1.0;
  }
#endif
};

} // namespace facebook::react
