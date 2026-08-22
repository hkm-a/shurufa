param(
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot)
)
# 重新生成三份构建期产物：
#   schemas/shurufa_t9.dict.yaml
#   schemas/jianpin_index.txt
#   schemas/rime_ice_nojianpin.schema.yaml + .dict.yaml（词典壳）
# 阶段 3 目标：这些文件已出库（gitignore），由 CI/构建前调用本脚本生成。
# 同时更新 schemas/generated-files.sha256 小清单，CI 以该清单校验一致性。
$ErrorActionPreference = "Stop"
python (Join-Path $PSScriptRoot 'regenerate-generated.py') $RepoRoot
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Host "已重新生成三份产物并更新 sha256 清单"
