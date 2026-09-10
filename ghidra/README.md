# Ghidra + ghidra-cli container

Headless Ghidra with [`ghidra-cli`](https://github.com/akiselev/ghidra-cli),
used to reverse-engineer the AYANEO gamepad protocol from `AYASpaceCef.exe`.
Findings: **[../docs/GAMEPAD-PROTOCOL.md](../docs/GAMEPAD-PROTOCOL.md)**.

In a container because `ghidra-cli` needs a **full JDK 21+** (it compiles a Java
bridge at runtime) and Ghidra 12 rejects newer JDKs — awkward to satisfy on a
rolling distro that ships only a JRE.

## Build and run

```bash
docker build -t ghidra-cli .
mkdir -p work project
cp /path/to/AYASpaceCef.exe work/

docker run -d --name ghidra \
    -v "$PWD/work":/work -v "$PWD/project":/project \
    --memory 12g ghidra-cli sleep infinity

docker exec ghidra ghidra import /work/AYASpaceCef.exe --project /project/aya
```

Import plus full analysis takes about five minutes for this binary (53 370
functions) and leaves a 47 MB program database under `project/`. Keeping the
container alive with `sleep infinity` avoids paying JVM startup on every query.

## Querying

Every command needs `--project` and `--program`; there is no default project.

```bash
D="--project /project/aya --program AYASpaceCef.exe"

docker exec ghidra ghidra decompile 0x140305a20 $D
docker exec ghidra ghidra function x-refs 0x1403397a0 $D
docker exec ghidra ghidra function list $D --limit 0
```

Output is JSON; `decompile` returns the C in a `code` field, so pipe it through
something that unescapes newlines:

```bash
docker exec ghidra ghidra decompile 0x140339280 $D \
  | python3 -c 'import sys,json; [print(e["code"]) for e in json.load(sys.stdin)]'
```

## Notes

* The installed binary is called **`ghidra`**, not `ghidra-cli`.
* `ghidra project create NAME --path DIR` does not take `--path`; just pass
  `--project /path/to/proj` to `import` and it initialises the project there.
* A killed `docker exec` leaves `project/*.lock` behind. Delete the lock files
  before reopening the project.
* `AYASpaceCef.exe` is not redistributed here. Extract it from an AYASpace
  installer; the analysed build was 3.2.0.4, `ImageBase 0x140000000`.
