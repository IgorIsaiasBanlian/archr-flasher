# ArchR Flasher 2.0: Release Notes

> Desktop app (Tauri 2 + Rust + JS) para gravar imagens ArchR em cartões SD,
> com gestão de overlay de painel integrada ao gerador on-line.

## Destaques

- **Geração de DTBO sai do flasher** e migra inteiramente para o gerador on-line: o app agora pede o `mipi-panel.dtbo` produzido em `https://arch-r.io/overlay-generator/` e o copia para `/flash/overlays/` durante a gravação.
- **Fluxo redesenhado em 5 passos**: Console → Imagem → Overlay → SD → Flash. Modo "Overlay" separado, para aplicar/atualizar overlay em um cartão já gravado.
- **Net -5.000 LOC**: 4 módulos Rust apagados, lib Python `fdt/` bundled removida, dependência `fdt = "0.1"` removida do `Cargo.toml`.
- **Suporte completo em 5 idiomas** (en, pt-BR, es, zh, ru): 84 chaves cada, sincronizadas.
- **XSS fix** em `buildFlashSummary` e `buildOverlaySummary` (nomes de arquivo Linux/macOS podem conter `<script>`).

---

## Por categoria

### Mudança estratégica: overlay vem do site

A geração de overlay deixou de ser responsabilidade do flasher. O fluxo agora é:

1. **Console**: usuário escolhe Original / Clone / Soysauce.
2. **Imagem**: download da release ou seleção de arquivo local.
3. **Overlay**:
   - Botão "Open the Overlay Generator" abre `https://arch-r.io/overlay-generator/` em nova janela.
   - File picker para o `mipi-panel.dtbo` baixado.
   - Checkbox "Use default overlay (skip)" para hardware que funciona com o overlay padrão da imagem.
4. **SD Card**: seleção do dispositivo (já filtrado para removíveis).
5. **Flash**: grava a imagem e copia o overlay para `/flash/overlays/mipi-panel.dtbo`.

Modo "Overlay" (na sidebar): aplica/atualiza overlay em um cartão SD ArchR já gravado, sem reflashar.

### Backend (Rust)

- `flash_image(image_path, device, overlay_path, variant)`: nova assinatura. `overlay_path` vazio = manter o overlay padrão da imagem; preenchido = copiar verbatim para `/flash/overlays/mipi-panel.dtbo`.
- `apply_custom_overlay(boot_path, dtbo_path)`: substitui o antigo multi-knob `apply_overlay_with_config`.
- `OverlayStatus` reduzido a `{boot_path, has_archr, current_overlay, current_panel_name, variant}`.
- Módulos apagados: `dtb_to_overlay.rs`, `dtbo_builder.rs`, `panel_config.rs`, `panels.rs`.
- Script bundled apagado: `src-tauri/scripts/archr-dtbo.py` + lib `fdt/`.
- Cargo.toml: `fdt = "0.1"` removido.
- `cargo check` limpo.

### Frontend (JS + HTML)

- `index.html` reescrito com 5 passos (modo Flash) e 3 passos (modo Overlay).
- `main.js`:
  - `pickOverlayDtbo()`: file picker via `Tauri.dialog.open()` com filtro `.dtbo`.
  - `buildFlashSummary` e `buildOverlaySummary` reescritos com `createElement` + `textContent` em vez de `innerHTML`. Corrige caminho de self-XSS via nome de arquivo: filenames em Linux/macOS podem conter caracteres `<` e `>` literais.

### Internacionalização (5 arquivos)

`en.json`, `pt-BR.json`, `es.json`, `zh.json`, `ru.json`: **84 chaves cada, sincronizadas**.

Chaves novas para o fluxo de overlay externo:
- `step_overlay`
- `overlay_intro`
- `open_generator`
- `pick_overlay`
- `no_overlay_picked`
- `overlay_will_skip`
- `overlay_skip_hint`
- `overlay_skip_label`
- `overlay_default_image`

Chaves órfãs removidas: `step_panel`, `step_customize`, `custom_dtb`, `custom_dtb_*`, `panel_disclaimer`, `panel_disclaimer_ok`, `panel_hint`, `online_generator`, `customizations`, `rotation`, `invert_lstick`, `invert_rstick`, `hp_invert`, `select_panel`, `select_panel_title`, `overlay_overlay_file`, `overlay_new_panel`, `overlay_current`, etc.

### Performance e UX

- `dd` substituído por helper `pwrite` nativo: progresso ao vivo via `/proc/$PID/io`, sem precisar de pipes.
- Live MB / total ao lado da barra, label da fase abaixo (descompactando, gravando, verificando).
- Verificação de integridade pós-gravação com mensagens específicas por tipo de falha.
- udisks2 suspenso durante o flash para evitar corrupção por automount intermediário.
- Wipe de assinaturas (`wipefs`) antes da gravação, invalidação de cache de imagens descompactadas quando a fonte muda.

---

## Mudanças incompatíveis (atenção)

- **Flasher 1.x não consegue mais gravar imagens ArchR 2.0 com overlay customizado** (a interface de seleção de painel saiu do app). Use o Flasher 2.0 ou monte a partição BOOT manualmente.
- **O fluxo de geração de painel foi removido**: usuários que dependiam de selecionar painel dentro do flasher precisam agora ir até o gerador on-line, baixar o `mipi-panel.dtbo` e selecioná-lo no passo 3.

---

## Compatibilidade

- **Imagens**: ArchR 2.0+. Imagens 1.x ainda gravam com o flasher 2.0 (o overlay padrão da imagem é mantido).
- **Plataformas**: Linux (AppImage), Windows, macOS.
- **Permissões**: pkexec (Linux), osascript (macOS), powershell elevation (Windows).

---

## Como construir

```bash
cd /media/disco-local/ArchR/archr-flasher/src-tauri
cargo tauri build
```

Bundle de release sai em `src-tauri/target/release/bundle/`. Para AppImage no Linux, o build usa o helper interno do Tauri. O updater está configurado para `https://github.com/archr-linux/archr-flasher/releases/latest`.

---

## Próximos passos da release

1. Bump da versão em `src-tauri/Cargo.toml` (atualmente 1.3.3 → 2.0.0).
2. Build em CI para Linux/Windows/macOS.
3. Smoke test: gravar imagem ArchR 2.0 em Original, Clone e Soysauce, com overlay gerado pelo site.
4. Tag `2.0.0`.
5. Upload dos bundles para GitHub Releases.
6. Anúncio no site (o updater pega a release automaticamente para usuários já em 1.3.x).

---

## Créditos

ArchR Flasher é construído com Tauri 2, Rust, JS/HTML/CSS. Agradecimentos aos colaboradores que testaram as builds RC em hardware diverso e relataram bugs de SD lento, falhas de permissão e regressões de UI.
