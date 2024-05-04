# Ore CLI with Nvidia GPU Support

A command line interface for the Ore program to utilize Nvidia GPU's. 


Built by [@BenjaSOL](https://x.com/benjasol_) & [@KaedonsCrypto](https://x.com/KaedonsCrypto)

## Building

To build the Ore CLI, you will need to have the Rust programming language installed. You can install Rust by following the instructions on the [Rust website](https://www.rust-lang.org/tools/install).

You must have CUDA installed 

```sh
export CUDA_VISIBLE_DEVICES=<GPU_INDEX>
```

Windows users

```sh
nvcc windows.cu -o windows
```

Linux users

```sh
nvcc linux.cu -o linux
```

Take the path to the executable that was just created and pass it as an environment variable.

```sh
export PATH_TO_EXE=./linux
```

Once you have Rust installed, you can build the Ore CLI by running the following command:

```sh
cargo build --release
```


```sh
./target/release/ore.exe --rpc "" --priority-fee 1 --keypair 'path to keypair' mine --threads 4
```

You will now run your hashing on the GPU instead of the CPU!

Donations in ORE or SOL: EVK3M6Cth3uPZcATCtnZ16mqArSNXt5oC6kcmakwXudb

## Credits

[ORE Miner](https://github.com/tonyke-bot/ore-miner)

[ORE CLI](https://github.com/HardhatChad/ore-cli)
