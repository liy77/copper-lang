<img src="./assets/copperlang.png"/>

# Example Usage

```crs
import * from std.io

name = input!("What's your name?")
println!("Your name is $name")
```

<p align="center">
    <img src="./assets/cforge.png" width=300 height=300 />
</p>

# Compiling your project
```sh
cforge -i ./src
```

### Flags
| Flag        | Full Name     | Description                                      | Example Usage                  |
|-------------|---------------|--------------------------------------------------|--------------------------------|
| `-i`        | `--input`     | Specifies the output file.                       | `cforge -o result.exe`         |
| `-od`       | `--outdir`    | Specifies the output directory.                  | `cforge -od /path/to/directory`|
| `-t`        | `--target`    | Specifies the operation system of target         | `cforge -t windows`            |
| `-c`        | `--compile`   | Indicates that the program should be compiled    | `cforge -c -i main.crs`        |
|             | `--clean`     | Cleans the output directory                      | `cforge clean`                 |


# Running
```sh
cforge run ./src/main.crs
```

## Some Features
### Classes
```
class Person {
    name: String
    name2: String
    inttest: i32

    Person(name: String, name2: String) {
        self.name = name
        self.name2 = "Carlos".to_string()
        self.inttest = 32
    }

    void test(self) { 
        println!("{} {}", self.name, self.name2); 
    }
}

Person::new(strfy("Brian"), strfy("Rhudy")).test()
```
