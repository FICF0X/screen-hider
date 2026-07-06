# Screen Hider

Oculta ventanas específicas de la **compartición de pantalla y las grabaciones**, en tiempo real. La ventana sigue visible para vos en tu monitor, pero **desaparece de lo que ven los demás** en una videollamada (Meet, Zoom, Teams, Discord…).

Ideal para dejar a la vista un Excel, un documento o notas mientras compartís pantalla completa, sin que el resto lo vea.

---

## Características

- 📋 Lista de ventanas abiertas **agrupadas por aplicación**, igual que el selector de "compartir pantalla".
- 👁️ **Ocultar / mostrar** cualquier ventana en tiempo real, con un clic.
- 🧩 Soporte para aplicaciones de **32 y 64 bits**.
- ⌨️ **Atajo global `Ctrl+Alt+H`** para ocultar/mostrar todo de golpe (botón de pánico) — funciona incluso con la app en segundo plano.
- 💾 **Recuerda entre reinicios** lo que ocultaste manualmente.
- 🌐 Interfaz en **Español / Inglés**.
- 🫥 Puede **ocultarse a sí misma** de la captura.

---

## Descargar y usar

1. Bajá el `.zip` de la última **[Release](../../releases/latest)**.
2. Descomprimí la carpeta completa (los 3 archivos tienen que quedar juntos).
3. Ejecutá **`ui.exe`**.

### ⚠️ Aviso sobre el antivirus

Screen Hider usa **inyección de DLL**, una técnica legítima pero que los antivirus asocian con software malicioso. Es esperable que:

- **SmartScreen** muestre *"Windows protegió tu PC – editor desconocido"* → clic en **Más información → Ejecutar de todas formas**.
- **Windows Defender** lo marque y lo ponga en cuarentena → hay que **permitir / restaurar** el archivo.

No es un virus: el código es abierto y podés revisar exactamente qué hace. El aviso aparece porque el ejecutable no está firmado y por la técnica de inyección.

---

## Cómo funciona

Usa la API de Windows `SetWindowDisplayAffinity` con el flag `WDA_EXCLUDEFROMCAPTURE`. Como esa API **solo funciona sobre ventanas del propio proceso**, Screen Hider inyecta una pequeña DLL en el proceso dueño de la ventana y aplica el flag desde adentro.

---

## Compilar desde el código

Requisitos:
- **Rust** (usa el toolchain *nightly*, que se fija solo vía `rust-toolchain.toml`).
- **Build Tools de C++** de Visual Studio (para el linker MSVC).
- El target de 32 bits: `rustup target add i686-pc-windows-msvc`.

```bash
./build.sh release
```

Genera en `target/release/`: `ui.exe`, `payload64.dll` y `payload32.dll`.

---

## Arquitectura

Workspace de Rust con cuatro crates:

| Crate      | Rol                                                              |
|------------|-----------------------------------------------------------------|
| `payload`  | DLL (cdylib) que se inyecta y llama a `SetWindowDisplayAffinity`.|
| `engine`   | Enumeración de ventanas + inyección (crate `dll-syringe`).      |
| `injector` | Interfaz de línea de comandos.                                  |
| `ui`       | Interfaz gráfica (`egui` / `eframe`).                            |

---

## Licencia

[MIT](LICENSE)
