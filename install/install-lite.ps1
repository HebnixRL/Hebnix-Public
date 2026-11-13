# Hebnix Lite one-line installer
#
#   irm https://raw.githubusercontent.com/Hebbins/Hebnix-Setup/main/install-lite.ps1 | iex
#
# All the logic lives in install.ps1; this just runs it with -Edition Lite.

& ([scriptblock]::Create((Invoke-RestMethod -UseBasicParsing 'https://raw.githubusercontent.com/Hebbins/Hebnix-Setup/main/install.ps1'))) -Edition Lite
