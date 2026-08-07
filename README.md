# Screen Sharing

A screen-sharing applet for the [COSMIC](https://system76.com/cosmic) desktop. Share your screen to wired displays (HDMI / VGA / DisplayPort) and wireless FCast receivers from a single place in the panel.

## Installing

Download the `.deb`, `.rpm`, or tarball from the [releases page](https://github.com/nalladev/cosmic-ext-applet-screen-sharing/releases/latest), or install from the COSMIC Store.

Then restart the panel and add the applet:

```sh
pkill cosmic-panel
```

Open **Settings → Desktop → Panel → Applets** and enable **Screen Sharing**.

## Building from source

Clone the repository and install with [just](https://github.com/casey/just):

```sh
git clone https://github.com/nalladev/cosmic-ext-applet-screen-sharing
cd cosmic-ext-applet-screen-sharing
just build-release
sudo just install
```

Then restart the panel and add the applet as above.

## Development

```sh
just build-release       # Release build
just build-debug         # Debug build
just run                 # Run standalone for testing
sudo just install        # Install system-wide
just check               # Type-check (cargo check)
just lint                # Run clippy lints
RUST_LOG=debug just run  # Run with verbose debug logging
just flatpak-install     # Build and install the Flatpak from the working tree
```

## Contributing

Contributions are welcome. Feel free to open issues or submit pull requests.

## License

[MPL-2.0](LICENSE)
