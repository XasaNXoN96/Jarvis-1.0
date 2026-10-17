#!/bin/sh
# The first start makes the database and the secret files (init does nothing to files that exist); then the command runs.
set -e
if [ ! -f "$JARVIS_LS_SIGNING_KEY_FILE" ] || [ ! -f "$JARVIS_LS_PEPPER_FILE" ] || [ ! -f "$JARVIS_LS_DB" ]; then
  jarvis-license-server init
fi
exec jarvis-license-server "$@"
