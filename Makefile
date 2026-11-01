# probe-wink: one entry point per part. Each target regenerates its outputs from source.
#
#   make pcb         schematic -> ERC/netlist check -> board (place, route, DRC, layout checks) -> fab files
#   make enclosure   board geometry -> housing (STEP/STL) -> enclosure checks -> renders
#   make firmware    ESP32-C3 module firmware (needs WINK_ESPNOW_KEY / WINK_BLE_PASSKEY or firmware/.envrc)
#   make devkit      the same firmware for an ESP32 DEVKITV1 test bench
#   make android     Android app (debug APK)
#   make test        firmware core tests + Android unit tests
#   make all         pcb, enclosure, firmware, android

PY      ?= python3
ENC_PY  ?= enclosure/.venv/bin/python
SECRETS  = $(if $(wildcard firmware/.envrc),. ./.envrc &&,)

.PHONY: all pcb enclosure firmware devkit android test

all: pcb enclosure firmware android

pcb:
	$(PY) pcb/schematic.py
	$(PY) pcb/check.py
	$(PY) pcb/board.py
	$(PY) pcb/fab.py

enclosure:
	$(PY) pcb/export_enclosure.py
	$(ENC_PY) enclosure/housing.py
	$(ENC_PY) enclosure/check.py
	$(ENC_PY) enclosure/render.py

firmware:
	cd firmware && $(SECRETS) cargo build -p wink-app --release

devkit:
	firmware/scripts/devkit build

android:
	cd android && ./gradlew assembleDebug

test:
	cd firmware/wink-core && cargo test --target x86_64-unknown-linux-gnu
	cd android && ./gradlew testDebugUnitTest
