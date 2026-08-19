$ErrorActionPreference = 'Stop'

$packageName = 'ai-usage-tui'
$url = 'https://github.com/SophanaSok/ai-usage-tui/releases/download/__TAG__/ai-usage-tui-__TAG__-x86_64-windows.zip'
$checksum = '__WINDOWS_SHA256__'
$checksumType = 'sha256'

Install-ChocolateyZipPackage -PackageName $packageName -Url $url -Checksum $checksum -ChecksumType $checksumType -UnzipLocation "$(Split-Path -Parent $MyInvocation.MyCommand.Definition)"
