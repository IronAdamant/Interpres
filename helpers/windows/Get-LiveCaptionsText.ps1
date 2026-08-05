# Read Windows 11 Live Captions text via UI Automation (no third-party modules).
# Prints the current CaptionsTextBlock Name to stdout.
# Exit 2 if Live Captions window not found.

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$class = 'LiveCaptionsDesktopWindow'
$root = [System.Windows.Automation.AutomationElement]::RootElement
$cond = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ClassNameProperty, $class)
$win = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
if (-not $win) {
    # also search descendants
    $win = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
}
if (-not $win) {
    [Console]::Error.WriteLine('STATUS lc=stopped reason=window_not_found')
    exit 2
}

function Find-ByAutomationId($parent, $id) {
    $c = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::AutomationIdProperty, $id)
    return $parent.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $c)
}

$el = Find-ByAutomationId $win 'CaptionsTextBlock'
if (-not $el) { $el = Find-ByAutomationId $win 'CaptionsScrollViewer' }
if (-not $el) {
    [Console]::Error.WriteLine('STATUS lc=degraded reason=automation_id_missing')
    exit 4
}

$name = $el.Current.Name
if ($null -eq $name) { $name = '' }
[Console]::Out.WriteLine($name)
exit 0
