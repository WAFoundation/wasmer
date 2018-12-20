<p align="center"><a href="https://wasmer.io" target="_blank" rel="noopener noreferrer"><img width="400" src="https://raw.githubusercontent.com/wasmerio/wasmer/master/logo.png" alt="Wasmer logo"></a></p>

<p align="center">
  <a href="https://circleci.com/gh/wasmerio/wasmer/"><img src="https://img.shields.io/circleci/project/github/wasmerio/wasmer/master.svg" alt="Build Status"></a>
  <a href="https://github.com/wasmerio/wasmer/blob/master/LICENSE"><img src="https://img.shields.io/github/license/wasmerio/wasmer.svg" alt="License"></a>
</p>

## Introduction

[Wasmer](https://wasmer.io/) is a Standalone JIT WebAssembly runtime, aiming to be fully compatible with Emscripten, Rust and Go.

Install Wasmer with:

```sh
curl https://get.wasmer.io -sSfL | sh
```

### Usage

`wasmer` can execute both the standard binary format (`.wasm`) and the text
format defined by the WebAssembly reference interpreter (`.wat`).

Once installed, you will be able to run any wasm module (_including Nginx!_):

```sh
wasmer run examples/nginx/nginx.wasm -- -p examples/nginx -c nginx.conf
```

## Building & Running

To build this project you will need Rust and Cargo.

```sh
# checkout code
git clone https://github.com/wasmerio/wasmer.git
cd wasmer

# install tools
# make sure that `python` is accessible.
cargo install
```

## Testing

Thanks to [spectests](https://github.com/wasmerio/wasmer/tree/master/spectests) we can assure 100% compatibility with the WebAssembly spec test suite.

Tests can be run with:

```sh
make test
```

If you need to re-generate the Rust tests from the spectests
you can run:

```sh
make spectests
```

## Roadmap

Wasmer is an open project guided by strong principles, aiming to be modular, flexible and fast. It is open to the community to help set its direction.

Below are some of the goals (written with order) of this project:

- [x] It should be 100% compatible with the [WebAssembly Spectest](https://github.com/wasmerio/wasmer/tree/master/spectests)
- [x] It should be fast _(partially achieved)_
- [ ] Support Emscripten calls _(on the works)_
- [ ] Support Rust ABI calls


## Architecture

If you would like to know how Wasmer works under the hood, please visit our [ARCHITECTURE](https://github.com/wasmerio/wasmer/blob/master/ARCHITECTURE.md) document.

## License

MIT/Apache-2.0

<small>[Attributions](./ATTRIBUTIONS.md)</small>.
