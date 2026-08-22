param()
# 生成「无简拼」变体方案（M10 困难项简拼开关的部署期替代）：
# librime 1.17 speller/algebra 不支持条件规则（option@jianpin: 实测
# Error loading formula #13），无法热开关简拼；改为生成去掉 abbrev 规则的
# rime_ice_nojianpin.schema.yaml，由设置中心方案页切换 + 重新部署生效。
#
# 阶段3曾评估 *.custom.yaml patch 覆盖层：Rime 的 __include 可跨文件引用
# 节点，但无法在列表中精准删除 abbrev 条目；在未验证 schema 级继承前，
# 继续保留“生成完整副本”策略，避免破坏 20 个引擎集成测试的安全网。
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$src = Join-Path $root "schemas\rime_ice.schema.yaml"
$dst = Join-Path $root "schemas\rime_ice_nojianpin.schema.yaml"
$lines = Get-Content $src -Encoding UTF8
$kept = New-Object System.Collections.Generic.List[string]
$removed = 0
foreach ($line in $lines) {
  if ($line -match "^\s+- abbrev/") { $removed++; continue }
  if ($line -eq "  schema_id: rime_ice") { $kept.Add("  schema_id: rime_ice_nojianpin"); continue }
  if ($line -eq "  name: 雾凇拼音") { $kept.Add("  name: 雾凇拼音（无简拼）"); continue }
  $kept.Add($line)
}
$enc = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllLines($dst, $kept.ToArray(), $enc)
Write-Host ("已生成 {0}（删除 abbrev 规则 {1} 行）" -f $dst, $removed)
