#!/bin/sh
set -eu

dogfood-cli db migrate
if [ "${SEED_ON_START:-false}" = "true" ]; then
  dogfood-cli db seed --reset
fi
exec dogfood-cli start
