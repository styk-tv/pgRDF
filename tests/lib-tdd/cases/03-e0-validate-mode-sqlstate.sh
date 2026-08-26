#!/usr/bin/env bash
# TDD C1-1/E0 · validate unknown-mode gate SQLSTATE (the fail-closed mode gate)
# TARGET  22023 invalid_parameter_value     PREDICTED TODAY  XX000 (PL-6, shacl.rs:330)
# NOTE: E0's scope guard protects validate's REPORT payload; this is its ERROR path only.
. "$(dirname "$0")/../lib.sh"
st="$(sqlstate "SELECT pgrdf.validate(1, 2, mode => 'lib-tdd-nonsense')")"
case "$st" in
  22023) green "unknown mode raises 22023 invalid_parameter_value" ;;
  XX000) red   "unknown mode is XX000 internal_error — E0 unimplemented" ;;
  NONE)  broken "unknown mode did not error — the fail-closed mode gate is GONE (#regression)" ;;
  *)     broken "unknown mode raised '$st'" ;;
esac
