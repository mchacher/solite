# Release notes

What changes for the users of each release, in their words. One section per release, newest first,
under a heading `## x.y.z (date)` (for example `## 0.2.0 (2026-12-01)`); `/solite-release` writes it
in the release commit, and `scripts/release.sh` takes the section whose heading is `## <version>`
alone or `## <version>` followed by a space and more text (`## 0.2.0` and `## 0.2.0 (date)`, never
`## 0.2.01`) as the text of the GitHub release and of `manifest.json`. A release with no section here is
refused before anything is built.

## 0.3.2 (2026-11-28)

The schedule recipe no longer squeezes the box's memory.

- **`schedule-on-off` installs with room to spare**: loading a recipe with many settings used to
  take most of the box's fast internal memory for a moment; the recipe's description is now read
  and served as text, and the box keeps over three quarters of that memory free.
- Nothing changes in what the recipes show or do.

## 0.3.1 (2026-11-28)

More memory to spare with four recipes installed.

- **Four recipes, with room left**: the box keeps much more of its fast internal memory free
  while recipes are installed, updated or removed (the small blocks of memory now go to the
  larger memory where they fit), so that Wi-Fi and the radio never run short.
- **The recipe catalogue** is read once into a compact form, and its texts in every language are
  served whole, as Sowel's (groups and choices included).
- **The box's log** gives the memory free at each step of a recipe's install, update or
  removal, failed steps included.

## 0.3.0 (2026-11-28)

Five more of Sowel's recipes, and room for larger ones.

- **New recipes from the catalogue**, Sowel's own (same settings and texts):
  - `motion-light-dimmable`: dimmable lights on motion, a brightness per time slot;
  - `schedule-on-off`: on and off at set times or at sunrise and sunset, up to 3 windows a day;
  - `state-trigger-light`: lights on for a set time when an equipment changes state (the gate
    opens at night);
  - `presence-thermostat`: a thermostat at comfort when someone is in the room, lower when it is
    empty, with night mode and preheating (its mode switch on the card comes later);
  - `state-watch`: an alarm when something stays in a state too long (a door left open).
- **Larger recipes, more of them**: a recipe may be 32 KB; up to 4 different recipes are
  installed, each used up to 4 times (16 in all). The box refuses an install it has no memory for.
- **Recipes know the home's local time and weekday**, from its time zone.
- **The recipe form** takes text settings, a data of an equipment to watch, and hides the settings
  that do not apply, as Sowel's; number settings are sent as numbers.
- **Fixes**: a radio that closes at once is reported as closed on macOS too.

## 0.2.1 (2026-11-28)

Recipes become scripts you install without a new firmware, and the box recognises many more kinds
of devices. (0.2.0 was tagged but never published: its build failed; 0.2.1 is the same release.)

- **Recipes from a catalogue.** A recipe is now a JavaScript script, Sowel's own, signed with the
  recipe key: the Plugins page of Administration lists the catalogue
  (`mchacher/solite-recipes`), installs, updates and removes a recipe, or takes a signed file. No
  recipe is built in any more: `motion-light` (Sowel's 1.1.2, same settings and texts) is
  installed from the catalogue. An instance whose recipe is missing says "Recipe not installed".
- **More devices, recognised from the Zigbee standard** (no list of models):
  - metering plugs: power (W), energy (kWh) and current (A);
  - buttons and remotes: their `action` (on, off, toggle, brightness moves, scene recalls), each
    press reaching the recipes;
  - shutters: their position and the open, close, stop and go-to-position orders;
  - thermostats and radiator valves: the measured temperature, the heating setpoint and setting
    it (5 to 30 °C).
- **A Restart button** in the interface; an update now checks that the PSRAM is there before it
  keeps the new image.
- **Steadier on the box**: the scripts' stack moved out of the internal RAM, so two recipes of
  eight instances each fit and installs are no longer refused after a few of them; tighter bounds
  for the largest homes; a request whose body never comes holds the server 2 s instead of 5; a
  quiet mains device is asked before it is called offline.
- **Interface**: the Administration block sits at the bottom left, as in Sowel.

## 0.2.0 (2026-11-28)

Recipes become scripts you install without a new firmware, and the box recognises many more kinds
of devices.

- **Recipes from a catalogue.** A recipe is now a JavaScript script, Sowel's own, signed with the
  recipe key: the Plugins page of Administration lists the catalogue
  (`mchacher/solite-recipes`), installs, updates and removes a recipe, or takes a signed file. No
  recipe is built in any more: `motion-light` (Sowel's 1.1.2, same settings and texts) is
  installed from the catalogue. An instance whose recipe is missing says "Recipe not installed".
- **More devices, recognised from the Zigbee standard** (no list of models):
  - metering plugs: power (W), energy (kWh) and current (A);
  - buttons and remotes: their `action` (on, off, toggle, brightness moves, scene recalls), each
    press reaching the recipes;
  - shutters: their position and the open, close, stop and go-to-position orders;
  - thermostats and radiator valves: the measured temperature, the heating setpoint and setting
    it (5 to 30 °C).
- **A Restart button** in the interface; an update now checks that the PSRAM is there before it
  keeps the new image.
- **Steadier on the box**: the scripts' stack moved out of the internal RAM, so two recipes of
  eight instances each fit and installs are no longer refused after a few of them; tighter bounds
  for the largest homes; a request whose body never comes holds the server 2 s instead of 5; a
  quiet mains device is asked before it is called offline.
- **Interface**: the Administration block sits at the bottom left, as in Sowel.

## 0.1.1 (2026-11-26)

A small release that proves the update path: a box running 0.1.0 checks GitHub, downloads this one
and installs it by itself. Nothing else changes.

## 0.1.0 (2026-11-26)

The first release: Solite runs by itself in a Sonoff Dongle Max, with no server, no Docker and no
cloud.

- It forms its own Zigbee network through the Dongle Max's radio, pairs sensors and relays, and
  keeps them across a restart.
- It organises the home in zones and equipments, with Sowel's names and JSON, and runs the
  `motion-light` recipe: motion switches the light on, then off after the delay.
- It serves Sowel's interface and API (live events included) from the box itself, with Sowel's top
  bar: the home's name, the sun times and the connection.
- It asks for a password at the first start, protects every route with it, and can be reset to
  factory settings.
- It backs the whole box up to one encrypted file and restores it.
- It restarts itself when its engine stops, and falls back to a safe mode if it keeps failing.
- It updates itself from a signed release: it checks for a newer version, downloads it, installs it
  in the spare slot and goes back to the old one if the new one does not start.

Known limits: the Wi-Fi is set from the serial console (`scripts/box.sh wifi`), the access point for
a first set-up and Ethernet are not there yet, and the box serves HTTP, not HTTPS.

