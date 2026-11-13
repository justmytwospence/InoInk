#!/bin/sh
# Runs the scripted emulator tour at every screen geometry InoInk targets; screenshots go to
# target/inoink-emulator-shots/<codename>. Look at them: nothing here judges the layout.
set -eu
cd "$(dirname "$0")/.."
for d in monza:390 spaBW:391 condor:389 frost:377 luna:382 pika:372 trilogy:320; do
	echo "== $d"
	scripts/build-inoink.sh --run-emulator --data "${1:-target/inoink-fixtures}" --device "$d"
done
