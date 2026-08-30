# Phase 2B local test plan

## Build

```powershell
cd <codexflow repo>\codexflow\phase2b
.\scripts\install.ps1 -TargetDir F:\codexflow-target
```

## Register TrinityR

```powershell
F:\codexflow-target\release\codexflow.exe project add TrinityR --root F:\TrinityR
F:\codexflow-target\release\codexflow.exe project list
```

If the existing Codex app project store already contains the same root under
another project, the command should fail rather than creating duplicate
ownership.

## Resolve by cwd

```powershell
cd F:\TrinityR
F:\codexflow-target\release\codexflow.exe project current
```

Expected: TrinityR.

## Launch

```powershell
F:\codexflow-target\release\codexflow.exe
```

Verify the created thread is assigned to the TrinityR project id.

## Multi-project

Create a disposable second repository and register it.

Verify that running CodexFlow from each root resolves a different project and
that neither project loads the other's project-specific AGENTS.md or skills.

## Regression

Run ordinary:

```powershell
codex
```

Expected: no CodexFlow project id is injected and ordinary session behavior is
unchanged.
