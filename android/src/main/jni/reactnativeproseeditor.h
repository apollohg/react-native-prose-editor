#pragma once

// Narrow Android-only include boundary for the prepared prose Fabric adapter.
// Keeping this separate prevents generated React Native headers from leaking into
// the renderer-neutral registry and Kotlin facade.
#include <react/renderer/components/PreparedProseViewer/PreparedProseMeasurementsManager.h>
