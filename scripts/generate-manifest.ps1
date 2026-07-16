[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $InputPath,
    [string] $OutputPath = '',
    [string] $CandidateVersion = '1.1.0-unified.2'
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $PSScriptRoot '..\manifest\unified_hands_manifest.json'
}

$groups = [ordered]@{
    'browser-sessions-navigation-contexts' = @(
        'browser_attach','browser_back','browser_close','browser_close_tab','browser_context_create',
        'browser_context_destroy','browser_context_list','browser_context_switch','browser_cookies',
        'browser_debug_launch','browser_forward','browser_launch','browser_list_tab','browser_navigate',
        'browser_new_tab','browser_reload','browser_smart_navigate','browser_status','browser_switch_tab',
        'hands_attach_lock_acquire','hands_attach_lock_release','hands_attach_lock_status','hands_navigate'
    )
    'dom-and-page-state-discovery' = @(
        'browser_a11y_find','browser_a11y_snapshot','browser_exists','browser_get_bounds',
        'browser_get_clickables','browser_get_forms','browser_get_html','browser_get_metrics',
        'browser_get_text','browser_get_url'
    )
    'browser-interaction-and-forms' = @(
        'browser_click','browser_fill_form','browser_focus','browser_hover','browser_press',
        'browser_scroll','browser_select','browser_submit_form','browser_type','element_drag',
        'file_upload','retry_click'
    )
    'web-retrieval-crawling-page-artifacts' = @(
        'browser_bulk_extract','browser_crawl','browser_extract_content','browser_http_scrape',
        'browser_iframe_extract','browser_js_extract','browser_map','browser_page_capture',
        'browser_page_dump','browser_scroll_collect','browser_smart_browse','hands_read_page'
    )
    'api-and-network-intelligence' = @(
        'browser_get_all_network','browser_get_network_log','browser_get_performance_log',
        'browser_learn_api','browser_route','browser_route_clear','browser_route_list',
        'browser_route_remove','hands_network_poll','hands_network_subscribe',
        'hands_network_subscriptions','hands_network_unsubscribe'
    )
    'browser-evaluation-waits-traces-evidence' = @(
        'browser_evaluate','browser_screenshot','browser_screenshot_burst','browser_trace_save',
        'browser_trace_start','browser_trace_stop','browser_verify_state','browser_verify_visual',
        'browser_wait_for','browser_wait_idle','browser_wait_stable'
    )
    'browser-scripting-planning-agentic-execution' = @(
        'browser_agent','browser_batch','browser_eval','browser_inject_script','browser_plan','browser_script'
    )
    'desktop-apps-windows-input-uia' = @(
        'drag','type_into_window','uia_app_launch','uia_batch','uia_click','uia_find','uia_focus_window',
        'uia_get_state','uia_hold_key','uia_key_press','uia_list_window','uia_poll_event','uia_read_value',
        'uia_scroll','uia_shortcut','uia_type','uia_watch','uia_window_move','uia_window_resize',
        'uia_window_snap','uia_window_state'
    )
    'vision-ocr-visual-perception' = @(
        'hands_capture','hands_scan_qr','read_screen_text','vision_analyze','vision_cache_stats',
        'vision_check_user_input','vision_diff','vision_find_template','vision_load_image','vision_ocr',
        'vision_ocr_capabilities','vision_ocr_fast','vision_screenshot','vision_screenshot_hidden_window',
        'vision_screenshot_ocr','vision_zoom','wait_for_visual','window_screenshot'
    )
    'unified-cross-surface-actions-verification' = @(
        'find_and_click','hands_app_action','hands_click','hands_fill_form','hands_find',
        'hands_login_recovery','hands_type','hands_verify','hands_verify_expectations'
    )
    'workflow-scripting-run-telemetry' = @(
        'hands_script','hands_summarize_run'
    )
    'monitor-scope-and-topology' = @(
        'hands_monitor_scope'
    )
    'extensions-runtime-health' = @(
        'hands_health','hands_plugin_call','hands_plugin_list','hands_plugin_load',
        'hands_plugin_unload','status'
    )
}

$redundantFrontDoors = [ordered]@{
    'browser_page_dump' = 'browser_page_capture(mode="dom")'
    'browser_inject_script' = 'browser_eval(script="(function(){ <script> })()")'
    'retry_click' = 'browser_click(..., retry=clamp(max_attempts, 1, 3), retry_delay_ms=retry_delay_ms); for more than 3 attempts, use explicitly bounded repeated browser_click steps in hands_script or Workflow'
    'find_and_click' = 'optional uia_focus_window(title=window_title); hands_click(target=text) for left/right; for middle or exact coordinate control use hands_find(target=text) then uia_click(x, y, button, double_click); scan/focus windows explicitly to preserve legacy cross-window search'
    'read_screen_text' = 'hands_capture(target=window_title or "screen", window_title=window_title, ocr=true), or vision_screenshot_ocr for full-screen OCR'
    'type_into_window' = 'uia_focus_window(title); wait delay_ms; optional uia_click(x=click_x, y=click_y); uia_type(text)'
    'window_screenshot' = 'behind=true: vision_screenshot_hidden_window(title, save_path, ocr); behind=false: hands_capture(target=title, window_title=title, save_path=save_path, ocr=ocr)'
}

$folded = [ordered]@{
    'status' = 'hands_health'
    'vision_ocr_fast' = 'vision_ocr with automatic cache and backend metadata'
}

$workflowOwned = [ordered]@{
    'hands_self_record_lookup' = 'no exact Workflow equivalent; use explicit flow selection and current-site validation'
    'hands_self_record_start' = 'workflow:flow_record_start'
    'hands_self_record_stop_and_optimize' = 'workflow:flow_record_stop plus explicit optimization review'
}

$internalized = [ordered]@{
    'hands_attach_lock_acquire' = 'automatic browser attach coordination'
    'hands_attach_lock_release' = 'automatic browser detach coordination'
}

$groupByTool = @{}
foreach ($entry in $groups.GetEnumerator()) {
    foreach ($name in $entry.Value) {
        if ($groupByTool.ContainsKey($name)) {
            throw "Tool appears in multiple groups: $name"
        }
        $groupByTool[$name] = $entry.Key
    }
}

$source = Get-Content -Raw -LiteralPath $InputPath | ConvertFrom-Json
$sourceTools = @($source.tools)
$localUnique = @($sourceTools | Where-Object local).Count
$githubUnique = @($sourceTools | Where-Object github_main).Count
$shared = @($sourceTools | Where-Object { $_.local -and $_.github_main }).Count
$githubOnly = @($sourceTools | Where-Object { -not $_.local -and $_.github_main }).Count
$localOnly = @($sourceTools | Where-Object { $_.local -and -not $_.github_main }).Count
$knownDuplicateRegistrations = @($source.local_binary.known_duplicate_registrations)
$localRegistrations = [int] $source.local_binary.registrations_reported
$removedWorkflowFrontDoors = @(
    'hands_self_record_lookup',
    'hands_self_record_start',
    'hands_self_record_stop_and_optimize'
)
$canonicalSourceTools = @(
    $source.tools |
        Where-Object { $_.name -notin $removedWorkflowFrontDoors }
) + @(
    [pscustomobject][ordered]@{
        name = 'hands_monitor_scope'
        category = 'Runtime policy'
        local = $false
        github_main = $false
        status = 'Unified template only'
        ability = 'List displays and set, inspect, or clear the central fail-closed monitor boundary used by visual, UIA, and coordinate actions.'
        polytopia_role = 'Pins unattended play to one display and rejects actions that escape it.'
    }
)

$rows = foreach ($tool in $canonicalSourceTools) {
    if (-not $groupByTool.ContainsKey($tool.name)) {
        throw "Unassigned tool: $($tool.name)"
    }

    $disposition = 'canonical'
    $replacement = $null
    if ($redundantFrontDoors.Contains($tool.name)) {
        $disposition = 'compatibility_front_door'
        $replacement = $redundantFrontDoors[$tool.name]
    } elseif ($folded.Contains($tool.name)) {
        $disposition = 'fold_into_canonical'
        $replacement = $folded[$tool.name]
    } elseif ($workflowOwned.Contains($tool.name)) {
        $disposition = 'workflow_owned_default_hidden'
        $replacement = $workflowOwned[$tool.name]
    } elseif ($internalized.Contains($tool.name)) {
        $disposition = 'runtime_internal_default_hidden'
        $replacement = $internalized[$tool.name]
    }

    $strictExposed = -not $redundantFrontDoors.Contains($tool.name)
    $fullExposed = $strictExposed -and -not $folded.Contains($tool.name)
    $defaultExposed = $fullExposed -and -not $workflowOwned.Contains($tool.name) -and -not $internalized.Contains($tool.name)

    [pscustomobject][ordered]@{
        name = $tool.name
        collection = $groupByTool[$tool.name]
        local = [bool]$tool.local
        github_main = [bool]$tool.github_main
        source_status = $tool.status
        ability = $tool.ability
        disposition = $disposition
        replacement = $replacement
        exposed_strict = $strictExposed
        exposed_full = $fullExposed
        exposed_default = $defaultExposed
    }
}

$sourceNamesAll = @($canonicalSourceTools.name)
$sourceNames = @($sourceNamesAll | Sort-Object -Unique)
$sourceDuplicateNames = @(
    $sourceNamesAll |
        Group-Object |
        Where-Object Count -gt 1 |
        ForEach-Object Name
)
$assignedNames = @($groupByTool.Keys | Sort-Object -Unique)
$missing = @($sourceNames | Where-Object { $_ -notin $assignedNames })
$extra = @($assignedNames | Where-Object { $_ -notin $sourceNames })

$groupRows = foreach ($entry in $groups.GetEnumerator()) {
    $members = @($rows | Where-Object collection -eq $entry.Key)
    [pscustomobject][ordered]@{
        id = $entry.Key
        raw = $members.Count
        strict = @($members | Where-Object exposed_strict).Count
        full = @($members | Where-Object exposed_full).Count
        default = @($members | Where-Object exposed_default).Count
        tools = @($members.name)
    }
}

$output = [pscustomobject][ordered]@{
    schema = 'unified_hands_manifest_v1'
    generated_at = (Get-Date).ToString('o')
    snapshot = [pscustomobject][ordered]@{
        as_of = (Get-Date).ToString('yyyy-MM-dd')
        github_main_commit = $source.github_main_commit
        candidate = [pscustomobject][ordered]@{
            version = $CandidateVersion
        }
        audited_sources = [pscustomobject][ordered]@{
            github_main_unique_tools = $githubUnique
            local_live_unique_tools = $localUnique
            registrations_reported = $localRegistrations
            known_duplicate_registrations = $knownDuplicateRegistrations
        }
    }
    counts = [pscustomobject][ordered]@{
        raw_union_unique = $rows.Count
        local_registrations = $localRegistrations
        local_unique = $localUnique
        local_exact_duplicate_registrations = $knownDuplicateRegistrations.Count
        github_unique = $githubUnique
        shared = $shared
        github_only = $githubOnly
        local_only = $localOnly
        removed_workflow_front_doors = $removedWorkflowFrontDoors.Count
        template_only = 1
        strict_unique = @($rows | Where-Object exposed_strict).Count
        full_unique = @($rows | Where-Object exposed_full).Count
        default_unique = @($rows | Where-Object exposed_default).Count
        template_catalog_tool = 1
        compatibility_tools_list = $rows.Count + 1
        strict_tools_list = @($rows | Where-Object exposed_strict).Count + 1
        full_tools_list = @($rows | Where-Object exposed_full).Count + 1
        default_tools_list = @($rows | Where-Object exposed_default).Count + 1
    }
    profiles = [pscustomobject][ordered]@{
        compatibility = "All $($rows.Count) canonical union tools; exact duplicate registrations and three Workflow-owned self-record front doors removed."
        full = 'Canonical local surface; seven redundant compatibility front doors and two absorbed tools hidden.'
        default = 'Full minus manual attach-lock acquire/release; adds one catalog tool.'
    }
    collections = @($groupRows)
    replacements = [pscustomobject][ordered]@{
        redundant_front_doors = $redundantFrontDoors
        folded = $folded
        workflow_owned = $workflowOwned
        runtime_internalized = $internalized
    }
    tools = @($rows)
    checks = [pscustomobject][ordered]@{
        expected_union_unique = $sourceNames.Count
        raw_union_matches_expected = ($rows.Count -eq $sourceNames.Count)
        assigned_unique_matches_expected = ($assignedNames.Count -eq $sourceNames.Count)
        source_duplicate_names = $sourceDuplicateNames
        missing = $missing
        extra = $extra
        removed_workflow_front_doors_absent = (@($rows.name | Where-Object { $_ -in $removedWorkflowFrontDoors }).Count -eq 0)
        monitor_scope_present = ('hands_monitor_scope' -in @($rows.name))
        exact_cover = (
            $missing.Count -eq 0 -and
            $extra.Count -eq 0 -and
            $sourceDuplicateNames.Count -eq 0 -and
            $rows.Count -eq $sourceNames.Count -and
            $assignedNames.Count -eq $sourceNames.Count
        )
        group_raw_sum = ($groupRows | Measure-Object raw -Sum).Sum
        group_strict_sum = ($groupRows | Measure-Object strict -Sum).Sum
        group_full_sum = ($groupRows | Measure-Object full -Sum).Sum
        group_default_sum = ($groupRows | Measure-Object default -Sum).Sum
    }
}

$json = $output | ConvertTo-Json -Depth 12
$parent = Split-Path -Parent $OutputPath
if (-not (Test-Path -LiteralPath $parent)) {
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
}
[System.IO.File]::WriteAllText(
    $OutputPath,
    $json,
    [System.Text.UTF8Encoding]::new($false)
)

$output.counts | ConvertTo-Json -Compress
$output.checks | ConvertTo-Json -Compress
