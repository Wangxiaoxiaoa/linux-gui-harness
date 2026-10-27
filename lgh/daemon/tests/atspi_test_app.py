#!/usr/bin/env python3
"""Minimal GTK3 test app for AT-SPI integration tests."""
import sys
import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk


class App(Gtk.Window):
    def __init__(self):
        super().__init__(title="lgh-atspi-test")
        self.set_default_size(400, 200)

        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        box.set_margin_top(10)
        box.set_margin_bottom(10)
        box.set_margin_start(10)
        box.set_margin_end(10)
        self.add(box)

        self.entry = Gtk.Entry()
        self.entry.set_text("initial")
        self.entry.set_name("test-entry")
        box.pack_start(self.entry, False, True, 0)

        self.button = Gtk.Button(label="Click me")
        self.button.set_name("test-button")
        self.button.connect("clicked", self.on_click)
        box.pack_start(self.button, False, True, 0)

        self.label = Gtk.Label(label="not clicked")
        self.label.set_name("test-label")
        box.pack_start(self.label, False, True, 0)

        self.connect("destroy", Gtk.main_quit)
        self.show_all()

    def on_click(self, _):
        self.label.set_text("clicked")


if __name__ == "__main__":
    app = App()
    Gtk.main()
