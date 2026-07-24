$ErrorActionPreference = 'Stop'

$packageName = 'ai-usage-tui'
$url = 'https://github.com/SophanaSok/ai-usage-tui/releases/download/v0.2.0/ai-usage-tui-0.2.0-x86_64-windows.zip'
$checksum = 'PLACEHOLDER_SHA256'
$checksumType = 'sha256'

Install-ChocolateyZipPackage -PackageName $packageName -Url $url -Checksum $checksum -ChecksumType $checksumType -UnzipLocation "$(Split-Path -Parent $MyInvocation.MyCommand.Definition)"