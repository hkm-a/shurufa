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
# Windows runner 的 Python 按区域编码 stdout（如 cp1252），生成脚本任何非 ASCII
# 输出都会 UnicodeEncodeError；统一强制 UTF-8 模式（Python 3.7+）。
$env:PYTHONUTF8 = "1"
python (Join-Path $PSScriptRoot 'regenerate-generated.py') $RepoRoot
if ($LASTEXITCODE -ne 0) { throw "regenerate-generated.py 失败（退出码 $LASTEXITCODE）" }
Write-Host "已重新生成三份产物并更新 sha256 清单"
