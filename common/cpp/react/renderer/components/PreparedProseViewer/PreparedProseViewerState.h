#pragma once

#include <cstdint>

#ifdef ANDROID
#include <folly/dynamic.h>
#endif

namespace facebook::react {

struct PreparedProseViewerState final {
  uint64_t attachmentRevision{0};
  uint64_t nativeFontRevision{0};

#ifdef ANDROID
  PreparedProseViewerState() = default;

  PreparedProseViewerState(
      const PreparedProseViewerState& /*previousState*/,
      const folly::dynamic& data)
      : attachmentRevision(revisionValue(data, "attachmentRevision")),
        nativeFontRevision(revisionValue(data, "nativeFontRevision")) {}

  folly::dynamic getDynamic() const {
    return folly::dynamic::object("attachmentRevision", attachmentRevision)(
        "nativeFontRevision", nativeFontRevision);
  }

 private:
  static uint64_t revisionValue(const folly::dynamic& data, const char* key) {
    const auto value = data.getDefault(key, 0).asInt();
    return value < 0 ? 0 : static_cast<uint64_t>(value);
  }
#endif
};

} // namespace facebook::react
