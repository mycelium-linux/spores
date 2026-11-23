# Hue, as a Soil Spore

The Hue controller as server-rendered HTML: groups, lights, scenes, power,
brightness, colour and scene activation.

The domain library is `../lib/hue.soil`; this Spore owns only the page, the
forms and the routes. It does not talk to the bridge directly: `soil:http` does,
and the host substitutes the `default` credential for `__CREDENTIAL__`.

## Load order

```text
core/prelude.soil
core/ui.soil
core/url.soil
std/bindings/soil-http.soil
std/bindings/soil-vfs.soil
../lib/hue.soil
../lib/cli.soil
source/render.soil
source/routes.soil
source/config.soil
```

## Bridge address

A Spore cannot discover Hue. The host workspace supplies `hue/bridge`:

```text
http://192.168.1.10
```

`source/config.soil` reads it through `soil:vfs`; when it is absent it falls
back to `http://127.0.0.1:8000` for the fake bridge. The app key is not in the
workspace; `hue/config-home` names the host credential handle `default`.

## Routes

| Route | What |
| --- | --- |
| `GET /` | groups, lights and scenes |
| `GET /healthz` | liveness |
| `POST /power` | on/off/toggle a group or light |
| `POST /brightness` | bri 1..254 |
| `POST /colour` | `#RRGGBB` -> xy through `hue/color-hex` |
| `POST /scene` | activate a named scene on a group or light |

## Tests

```sh
SOIL=/path/to/soil ./tests/run.sh
```

The suite is pure: fixture JSON, no bridge and no socket. It checks the page
shell, all control forms, and escaping.
