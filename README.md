<html>
    <img src="./assets/copper-foundation.png"/>
</html>

# Stop using directly rust, use copper instead.

<img src="./assets/no-rust-use-copper-instead.png" width="60%">

```crs
import * from std.io

func void main() {
    name = input!("What's your name?")

    println!("Your name is $name")
}
```

# Compiling
```sh
cforge -o ./hello.exe
```

### Flags
| Flag        | Full Name     | Description                                      | Example Usage                  |
|-------------|---------------|--------------------------------------------------|--------------------------------|
| `-o`        | `--out`       | Specifies the output file.                       | `cforge -o result.exe`         |
| `-d`        | `--outdir`    | Specifies the output directory.                  | `cforge -d /path/to/directory` |
| `-t`        | `--target`    | Specifies the operation system of target         | `cforge -t windows`            |
| `-c`        | `--config`    | Specifies the configuration file used to compile | `cforge -c ./src/config.kson`  |


# Running
```sh
cforge run ./src/main.crs
```

## Some Features
### Native JSON and KSON Support
```crs
let json = json_{
    "some_key": "some value"
}

let kmodel = kmodel_`
    some_key: String
`

let kson = kson_`
    some_key = "some value"
`.use_model(kmodel)

println!("$json") // Prints json
println!("$kson") // Prints kson
println!("{}", kson.json()) // Prints json

let kmodel2 = kson_`
    some_key: Integer
`

kson.use_model(kmodel2) // Throws error, invalid property type for "some_key"
```