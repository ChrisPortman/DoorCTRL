# DoorCTRL

## Overview

A simple device that functions as a controller for an electric door strike.

Rather than use a in-door mounted smart lock that requires batteries, I have an electric strike that
locks/unlocks (I have it setup to be locked by default) with the supply of 12v.  Additionally, I
have a magnetic reed switch that detects open/close state.

The device integrates with Home Assistant via MQTT.

## Motivation

Its very likely that something like Tasmota would have been able to be configured to do this job.
The main motivations were primarily around gaining experience in firmware development in Rust.

## Features

* Basic web interface supporting device control and configuration.
* Device initial setup mode. Device hosts a WiFi access point to connect to and perform initial
configuration.  The device's web interface in this mode can be reached on *http://192.168.0.1*.
Note the device does not provide DHCP, so the client will need to statically configure an IP address
on 192.168.0.0/24.
* Home Assistant integration via [MQTT with
discovery](https://www.home-assistant.io/integrations/mqtt/#mqtt-discovery) per .  MQTT supports TLS
(no certificate validation).
* *Factory* reset with long button push.
* Status indicator with RGB LED.

### LED Status

The following statuses are indicated by the RGB:

* Solid Red: Initialising
* Flashing Amber: unconfigured/setup mode
* Flashing Green: WiFi connected, MQTT not connected
* Solid Green: WiFi connected, MQTT connected.

## Hardware

This has been developed using a ESP32C3 development kit board that has a WS2812 RGB Led connected on
GPIO8.

Other GPIO assignments are:

* **GPIO1**: Triggers the lock.  Configured to pull Low so that the lock is triggered by setting the pin
  high.  Note that "triggered" depends on how the strike is set.  I.e. whether it is locks when
  powered or unlocked when powered.
* **GPIO2**: Monitors the reed switch interpreted as door open/closed.  Configured to pull high so
  the door registers as closed when grounded.
* **GPIO3**: Reset switch.  If held for 5 seconds, the current configuration is deleted and the
  device resets into setup mode.

The door lock in use is a [Lockwood ES110 Electric Strike](https://www.lockweb.com.au/au/en/products/electromechanical-solutions/electric-strikes/es110-series-electric-strike).  The reed is a cheap generic read from JayCar.

## Project Structure

The project is a workspace containing the following 2 crates:

1. `firmware`: this is mostly code that is specific to the ESP32C3 target and won't compile on
   *x86_64*.
2. `doorctrl`: this is code that will compile on *x86_64* and can therefore be easily tested.  The
   command alias `cargo test_pc` will run tests in this crate.

Generally the strategy has been to push as much code to `doorctrl` and call it from `firmware` to
facilitate testing.

An additional crate [weblite](https://docs.rs/weblite/latest/weblite/) was built as part of this,
but then pulled out and published independently as a simple `no_std` web framework, http protocol
and web socket protocol implementation.

## Screen Shots

Door open and locked:

![Open and Locked](/docs/web_screenshot_open_locked.png)

Door closed and unlocked:

![Closed and Unlocked](/docs/web_screenshot_closed_unlocked.png)

Configuration settings:

![Configuration Settings](/docs/web_screenshot_configuration.png)

