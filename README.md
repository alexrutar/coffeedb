# Coffee brew and bean tracking

This is a basic CLI application for keeping track of coffee beans and brews.
Install with `cargo install --path .`

This program uses unstable clap dynamic completions, which work great on Fish shell but support is not as good in zsh or Bash; use at your own risk.
Install completions with
```sh
COMPLETE=fish coffeedb > /path/to/your/completions/folder/coffeedb.fish
```

This application is larely LLM generated and I don't recommend reading the code.
It works well for my own workflow, and it may be useful for you too.
Unless otherwise specified, the source code in this repository is [public domain](https://unlicense.org/).
Feel free to use and modify as you wish.


## Basic usage

There are two subcommands: `bean` and `brew`.
The `bean` subcommand manages beans tracked by the application.

### Adding beans

Add a new bean using `coffeedb bean add`, and also specifying the roast or brew type.

Each bean is assigned a unique label of the form `{E, F, O}[1-9][0-9]*`, with numbers assigned in monotonically increasing order.
For example, `coffeedb bean add filter` when successful will create a new bean with label `F1`.
List existing beans with `coffeedb bean list`.
You can get human-readable text with `coffeedb bean get`.
Other formats are supported with the `--format` flag, including JSON and TOML output, and SVG output to create a label.

### Adding brews

Once you have added at least one bean, you can add brews for that bean.
This is done with `coffeedb brew add`, and you must specify the bean label and the device used for the brew.
Devices are referred to using a short name of your choice.
If the brew device does not exist you will be prompted to add it manually.

For example, if you decided to brew coffee `F1` with a V60, you would run
```sh
coffeedb brew add F1 v60
```
Currently, three devices types are supported (pourover, immersion, and espresso).

After running the command, you will be prompted to fill in the brew information.
The required information depends on the device type.

### Listing brews

You can view brews using `coffeedb brew list`.
By default, this lists all brews in decreasing chronological order.
Specify a bean to only list brews using that bean.

### Archiving beans

If you run out of a bean, archive it with `coffeedb bean archive`.
This will prevent the bean from being suggested by autocompletion.
