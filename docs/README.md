# probe-wink documentation

Wink and alternation mod for the pop-up headlamps of the 1989–92 Ford Probe,
hardware rev C.

| Page                                | Contents                                                   |
|-------------------------------------|------------------------------------------------------------|
| [How it works](how-it-works.md)     | stock retractor logic, the swap relay, timing, master and slave, safety layers, sleep |
| [Installation](installation.md)     | factory connector and wire colours, Y-adapter, 1 A fuse, mounting, side jumper, first power-up |
| [Using it](using.md)                | waking the modules, phone, BLE remote, states, limits     |
| [Building](building.md)             | PCB at JLCPCB, the lead, printing the housing, firmware, Android app |
| [Testing](testing.md)               | devkit test bench, bench and car tests, what is untested  |
| [Design notes](design-notes.md)     | the main decisions and why                                |

Per-part details:

- [pcb/README.md](../pcb/README.md): circuit, layout, layout checks, assembly
- [firmware/README.md](../firmware/README.md): toolchain, pins, behaviour, BLE
  protocol, ESP-NOW authentication, limits
- [android/README.md](../android/README.md): the Android app
- `enclosure/housing.py`: housing concept, print settings and hardware list in
  its docstring

Manual page numbers refer to the Chilton Ford Probe 1989–92 Repair Manual
(No. 8266). The wiring diagrams are redrawn and simplified from it. Line
colours show wire function (red UP, blue DOWN, amber +12 V, grey ground), not
insulation colour.
