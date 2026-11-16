#!/bin/sh
# Source formatting only: this sample is never executed.
name="CodeStyles"
greet() { printf '%s\n' "Hello, $1"; }
for item in alpha beta; do
if [ -n "$item" ]; then greet "$item"; fi
done
case "$name" in
CodeStyles) printf '%s\n' 'ready';;
*) printf '%s\n' 'other';;
esac
