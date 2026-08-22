param(
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot)
)
# 重新生成三份构建期产物：
#   schemas/shurufa_t9.dict.yaml
#   schemas/jianpin_index.txt
#   schemas/rime_ice_nojianpin.schema.yaml
# 阶段 3 目标：这些文件最终应出库，由 CI/构建前调用本脚本生成。
$ErrorActionPreference = "Stop"
Push-Location $RepoRoot
try {
    python scripts/gen-t9-dict.py $RepoRoot
    python scripts/gen-jianpin-index.py (Join-Path $RepoRoot "schemas")
    pwsh -NoProfile -File scripts/gen-nojianpin-schema.ps1
    Write-Host "已重新生成三份产物"
} finally {
    Pop-Location
}
