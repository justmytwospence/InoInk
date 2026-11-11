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
LOCK=/tmp/inoink.lock
if ! mkdir "$LOCK" 2>/dev/null; then
	exit 0
fi

# Restore the Kobo interface exactly once, however we leave.
RESTORED=0
restore() {
	[ "$RESTORED" = 1 ] && return
	RESTORED=1
	[ "$ORIG_BPP" ] && ./bin/utils/fbdepth -q -d "$ORIG_BPP" 2>/dev/null
	rmdir "$LOCK" 2>/dev/null
	if [ -e /tmp/reboot ] ; then
		reboot
	elif [ -e /tmp/power_off ] ; then
		poweroff -f
	else
		./nickel.sh &
	fi
}
trap 'restore; exit 1' INT TERM HUP
trap restore EXIT

INOINK_SET_FRAMEBUFFER_DEPTH=1
# shellcheck disable=SC1091
[ -e "$INOINK_DATA/config.sh" ] && . "$INOINK_DATA/config.sh"

# shellcheck disable=SC2046
export $(grep -sE '^(INTERFACE|WIFI_MODULE|DBUS_SESSION_BUS_ADDRESS|NICKEL_HOME|LANG)=' /proc/"$(pidof -s nickel)"/environ)
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

export LD_LIBRARY_PATH="libs:${LD_LIBRARY_PATH}"

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

LIBC_FATAL_STDERR_=1 ./inoink >> "$LOG" 2>&1
exit 0
