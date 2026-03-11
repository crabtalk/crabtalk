#!/bin/sh
# Workaround for lance-linalg build.rs using -march=native when cross-compiling
# to x86_64-apple-darwin from ARM. Rewrites to a valid x86_64 architecture.
exec cc "${@//-march=native/-march=x86-64-v4}"
