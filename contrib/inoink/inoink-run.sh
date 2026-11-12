#! /bin/sh
# Runs InoInk with the normal Kobo interface stopped, then restores it. Adapted from Plato's
# launcher. Started by the dispatcher (../../current/inoink.sh); the working directory is this
# release directory, INOINK_DATA and INOINK_SETTINGS point into the data directory.

WORKDIR=$(dirname "$0")
cd "$WORKDIR" || exit 1

: "${INOINK_DATA:=../../data}"
export INOINK_DATA
: "${INOINK_SETTINGS:=$INOINK_DATA/Settings.toml}"
export INOINK_SETTINGS
mkdir -p "$INOINK_DATA"

# Do not start twice (a double tap on the menu entry would stop Nickel twice).
# A lock left by a run that died without cleaning up (its process is gone) is removed, so
# InoInk never silently refuses to start until the next reboot.
LOCK=/tmp/inoink.lock
if ! mkdir "$LOCK" 2>/dev/null; then
	OLD=$(cat "$LOCK/pid" 2>/dev/null)
	if [ -n "$OLD" ] && kill -0 "$OLD" 2>/dev/null; then
		echo "--- $(date) already running (pid $OLD); launch ignored" >> "$INOINK_DATA/inoink.log"
		exit 0
	fi
	echo "--- $(date) removing a stale lock (pid ${OLD:-unknown})" >> "$INOINK_DATA/inoink.log"
	rm -rf "$LOCK"
	mkdir "$LOCK" || exit 1
fi
echo $$ > "$LOCK/pid"

# Restore the Kobo interface exactly once, however we leave.
RESTORED=0
restore() {
	[ "$RESTORED" = 1 ] && return
	RESTORED=1
	[ "$ORIG_BPP" ] && ./bin/utils/fbdepth -q -d "$ORIG_BPP" 2>/dev/null
	rm -rf "$LOCK"
	if [ -e /tmp/reboot ] ; then
		sync
		reboot
	elif [ -e /tmp/power_off ] ; then
		sync
		poweroff -f
	elif [ "$PLATFORM" = "mt8113t-ntx" ] && [ -z "$INOINK_RESTART_NICKEL" ] ; then
		# MediaTek Kobos (Libra Colour, Clara Colour/BW): restarting Nickel after another app
		# has used the Wi-Fi or Bluetooth chip can crash the kernel while Nickel reloads the
		# chip's drivers (koreader/koreader#12739). The device then reboots uncleanly and may
		# come up in the setup screens. A clean reboot is the safe way back.
		echo "--- $(date) rebooting into the Kobo interface (MediaTek)" >> "$INOINK_DATA/inoink.log"
		sync
		reboot
	else
		REL_DIR=$(pwd)
		cd / && "$REL_DIR/nickel.sh" &
	fi
}
trap 'restore; exit 1' INT TERM HUP
trap restore EXIT

INOINK_SET_FRAMEBUFFER_DEPTH=1
# shellcheck disable=SC1091
[ -e "$INOINK_DATA/config.sh" ] && . "$INOINK_DATA/config.sh"

# shellcheck disable=SC2046
export $(grep -sE '^(INTERFACE|WIFI_MODULE|DBUS_SESSION_BUS_ADDRESS|NICKEL_HOME|LANG)=' /proc/"$(pidof -s nickel)"/environ)
# Start with the light and Wi-Fi state the Kobo was using (read before Nickel stops).
# INOINK_SYSROOT and INOINK_KOBO_CONF exist so the tests can stand in for the device.
SYSROOT=${INOINK_SYSROOT:-}
KOBO_CONF=${INOINK_KOBO_CONF:-/mnt/onboard/.kobo/Kobo/Kobo eReader.conf}
sys_white=
for f in actual_brightness brightness; do
	v=$(cat "$SYSROOT/sys/class/backlight/mxc_msp430.0/$f" 2>/dev/null)
	case "$v" in ''|*[!0-9]*) ;; *) sys_white=$v; break ;; esac
done
conf_level=$(sed -n 's/^FrontLightLevel=//p' "$KOBO_CONF" 2>/dev/null | head -n 1)
conf_state=$(sed -n 's/^FrontLightState=//p' "$KOBO_CONF" 2>/dev/null | head -n 1)
INOINK_FL_WHITE=
case "$sys_white" in
	''|0) ;;
	*) INOINK_FL_WHITE=$sys_white ;;
esac
if [ -z "$INOINK_FL_WHITE" ]; then
	if [ "$conf_state" = false ]; then
		INOINK_FL_WHITE=0
	else
		case "$conf_level" in ''|*[!0-9]*) ;; *) INOINK_FL_WHITE=$conf_level ;; esac
	fi
fi
INOINK_FL_COLOR=
for f in /sys/class/leds/aw99703-bl_FL1/color /sys/class/backlight/lm3630a_led/color /sys/class/backlight/tlc5947_bl/color; do
	if [ -r "$SYSROOT$f" ]; then INOINK_FL_COLOR=$(cat "$SYSROOT$f" 2>/dev/null); break; fi
done
INOINK_WIFI=0
if [ -d "$SYSROOT/sys/class/net/${INTERFACE:-wlan0}" ]; then
	if [ "$(cat "$SYSROOT/sys/class/net/${INTERFACE:-wlan0}/operstate" 2>/dev/null)" = up ] || [ -n "$(pidof wpa_supplicant 2>/dev/null)" ]; then INOINK_WIFI=1; fi
fi
LAUNCH_NOTE="light sys=$sys_white conf=$conf_level state=$conf_state -> $INOINK_FL_WHITE color=$INOINK_FL_COLOR wifi=$INOINK_WIFI"
export INOINK_FL_WHITE INOINK_FL_COLOR INOINK_WIFI
sync
killall -TERM nickel hindenburg sickel fickel adobehost foxitpdf iink dhcpcd-dbus dhcpcd fmon > /dev/null 2>&1

if [ -e /sys/class/leds/LED ] ; then
	LEDS_INTERFACE=/sys/class/leds/LED/brightness
	STANDARD_LEDS=1
elif [ -e /sys/class/leds/GLED ] ; then
	LEDS_INTERFACE=/sys/class/leds/GLED/brightness
	STANDARD_LEDS=1
elif [ -e /sys/class/leds/bd71828-green-led ] ; then
	LEDS_INTERFACE=/sys/class/leds/bd71828-green-led/brightness
	STANDARD_LEDS=1
elif [ -e /sys/devices/platform/ntx_led/lit ] ; then
	LEDS_INTERFACE=/sys/devices/platform/ntx_led/lit
	STANDARD_LEDS=0
elif [ -e /sys/devices/platform/pmic_light.1/lit ] ; then
	LEDS_INTERFACE=/sys/devices/platform/pmic_light.1/lit
	STANDARD_LEDS=0
fi

# Turn off the LEDs.
if [ -n "$LEDS_INTERFACE" ]; then
	if [ "$STANDARD_LEDS" -eq 1 ] ; then
		echo 0 > "$LEDS_INTERFACE"
	else
		for ch in 3 4 5; do
			echo "ch ${ch}" > "$LEDS_INTERFACE"
			echo "cur 1" > "$LEDS_INTERFACE"
			echo "dc 0" > "$LEDS_INTERFACE"
		done
	fi
fi

# Remount the SD card read-write if it is mounted read-only.
grep -q ' /mnt/sd .*[ ,]ro[ ,]' /proc/mounts && mount -o remount,rw /mnt/sd

# Environment used by scripts/usb-*.sh.
KOBO_TAG=/mnt/onboard/.kobo/version
if [ -e "$KOBO_TAG" ] ; then
	SERIAL_NUMBER=$(cut -f 1 -d ',' "$KOBO_TAG")
	FIRMWARE_VERSION=$(cut -f 3 -d ',' "$KOBO_TAG")
	MODEL_NUMBER=$(cut -f 6 -d ',' "$KOBO_TAG" | sed -e 's/^[0-]*//')

	case "$MODEL_NUMBER" in
		3[12]0)  PRODUCT_ID=0x4163 ;; 330) PRODUCT_ID=0x4173 ;; 340) PRODUCT_ID=0x4183 ;;
		350) PRODUCT_ID=0x4193 ;; 360) PRODUCT_ID=0x4203 ;; 370) PRODUCT_ID=0x4213 ;;
		371) PRODUCT_ID=0x4223 ;; 372) PRODUCT_ID=0x4224 ;; 373|381) PRODUCT_ID=0x4225 ;;
		374) PRODUCT_ID=0x4227 ;; 375) PRODUCT_ID=0x4226 ;; 376) PRODUCT_ID=0x4228 ;;
		377|380) PRODUCT_ID=0x4229 ;; 384) PRODUCT_ID=0x4232 ;; 382) PRODUCT_ID=0x4230 ;;
		387) PRODUCT_ID=0x4233 ;; 383) PRODUCT_ID=0x4231 ;; 388) PRODUCT_ID=0x4234 ;;
		386) PRODUCT_ID=0x4235 ;; 389) PRODUCT_ID=0x4236 ;;
		390) PRODUCT_ID=0x4237 ;; # Libra Colour
		393) PRODUCT_ID=0x4238 ;; # Clara Colour
		391|395) PRODUCT_ID=0x4239 ;;
		*) PRODUCT_ID=0x6666 ;;
	esac

	export SERIAL_NUMBER FIRMWARE_VERSION MODEL_NUMBER PRODUCT_ID
fi

export LD_LIBRARY_PATH="$(pwd)/libs${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

LOG="$INOINK_DATA/inoink.log"
[ -e "$LOG" ] && [ "$(stat -c '%s' "$LOG")" -gt $((1<<18)) ] && mv "$LOG" "$INOINK_DATA/inoink.old.log"

if [ "$INOINK_SET_FRAMEBUFFER_DEPTH" ] ; then
	case "${PRODUCT}:${MODEL_NUMBER}" in
		kraken:*|pixie:*|dragon:*|phoenix:*|dahlia:*|alyssum:*|pika:*|daylight:*|star:375|snow:374)
			[ -x ./bin/utils/fbdepth ] && ORIG_BPP=$(./bin/utils/fbdepth -g)
			;;
		*)
			unset ORIG_BPP
			;;
	esac
fi

[ "$ORIG_BPP" ] && ./bin/utils/fbdepth -q -d 8

echo "--- $(date) starting in $(pwd)" >> "$LOG"
echo "launch: $LAUNCH_NOTE" >> "$LOG"
# The Kobo's disk check renames files it rejects to FSCK*.000: record any in this release.
find . -iname 'FSCK*' -exec echo "warning: disk check leftover {}" \; >> "$LOG" 2>&1
LIBC_FATAL_STDERR_=1 ./inoink >> "$LOG" 2>&1
STATUS=$?
if [ "$STATUS" = 127 ]; then
	# The program could not be loaded: record why.
	{
		echo "--- loader failure; LD_LIBRARY_PATH=$LD_LIBRARY_PATH"
		ls -la libs
		LD_DEBUG=libs ./inoink 2>&1 | grep -iE 'find library|trying file|cannot|error' | head -60
	} >> "$LOG" 2>&1
fi
echo "--- exited with status $STATUS" >> "$LOG"
exit 0
