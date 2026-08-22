param()
# 生成「无简拼」变体方案（M10 困难项简拼开关的部署期替代）。
# 实际逻辑在 scripts/gen-nojianpin-schema.py（阶段 3 起统一走 Python，
# 保证 Windows/Linux 输出一致；本脚本保留为 PowerShell 兼容入口）。
#
# librime 1.17 speller/algebra 不支持条件规则（option@jianpin: 实测
# Error loading formula #13），无法热开关简拼；改为生成去掉 abbrev 规则的
# rime_ice_nojianpin.schema.yaml，由设置中心方案页切换 + 重新部署生效。
#
# 阶段3曾评估 *.custom.yaml patch 覆盖层：Rime 的 __include 可跨文件引用
# 节点，但无法在列表中精准删除 abbrev 条目；在未验证 schema 级继承前，
# 继续保留“生成完整副本”策略，避免破坏 20 个引擎集成测试的安全网。
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
python (Join-Path $PSScriptRoot 'gen-nojianpin-schema.py') $root
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
