#!/usr/bin/env bash

set -euo pipefail

local_env_file="ios-tests/.device-test.env"
if [[ -f "$local_env_file" ]]; then
  # shellcheck disable=SC1090
  source "$local_env_file"
fi

: "${IOS_DEVICE_ID:?Set IOS_DEVICE_ID or create ios-tests/.device-test.env}"
: "${IOS_DEVELOPMENT_TEAM:?Set IOS_DEVELOPMENT_TEAM or create ios-tests/.device-test.env}"

bash ./scripts/run-ios-tests.sh \
  --device-id "$IOS_DEVICE_ID" \
  --team-id "$IOS_DEVELOPMENT_TEAM" \
  "$@"
