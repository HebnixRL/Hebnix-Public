# Hebnix portable one-line installer
#
#   cd <the folder you want Hebnix in>
#   irm https://raw.githubusercontent.com/Hebbins/Hebnix-Setup/main/install-portable.ps1 | iex
#
# All the logic lives in install.ps1; this just runs it with -Portable, which installs
# into the current folder.

& ([scriptblock]::Create((Invoke-RestMethod -UseBasicParsing 'https://raw.githubusercontent.com/Hebbins/Hebnix-Setup/main/install.ps1'))) -Portable
