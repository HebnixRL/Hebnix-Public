# v27: hosting from the list only loads a map whose LAN slot copy is ready.
# Otherwise it logs HBXHOSTREQ and Hebnix copies the map into its slot, then
# starts the match itself (workshop_host's HebnixHost command), so a map is
# never hosted under its own package name, which other players can't match.
#
# Applies to the disassembled v26 movie, the shipped workshop_menu_ui.swf:
#   abcexport workshop_menu_ui.swf && rabcdasm workshop_menu_ui-0.abc
#   python v27_host.py workshop_menu_ui-0/menus/WorkshopMenu.class.asasm
#   rabcasm workshop_menu_ui-0/workshop_menu_ui-0.main.asasm
#   abcreplace workshop_menu_ui.swf 0 workshop_menu_ui-0/workshop_menu_ui-0.main.abc
import sys

path = sys.argv[1]
s = open(path, encoding='utf-8').read()


def rep(old, new):
    global s
    assert s.count(old) == 1, (s.count(old), old[:80])
    s = s.replace(old, new)


def op(name, arg=''):
    return '      ' + (name.ljust(20) + arg if arg else name) + '\n'


def q(n):
    return f'QName(PackageNamespace(""), "{n}")'


WS = 'QName(PackageNamespace("tagame.api"), "Workshop")'
DEV = 'QName(PackageNamespace("tagame.api"), "DevUtil")'
INDEX_OF = 'QName(Namespace("http://adobe.com/AS3/2006/builtin"), "indexOf")'


def log(tag, *value):
    return op('getlex', DEV) + op('pushstring', f'"{tag}"') + ''.join(value) + op('callpropvoid', f'{q("Log")}, 2') + '\n'


# the log reaches disk in 4 KB blocks, so pad it out for Hebnix to see the line now
PAD = ''.join(log('HBXPAD', op('pushstring', '"' + ' ' * 900 + '"')) for _ in range(6))

# 1 hbxHost: load the slot copy when it is ready, else ask Hebnix for it
rep(
    log('HBXHOST', op('getlocal1'), op('getproperty', q('FullPath')))
    + op('getlex', WS) + op('getlocal0') + op('getlocal1')
    + op('callproperty', f'{q("hbxPath")}, 1')
    + op('callpropvoid', f'{q("LoadWorkshopMap")}, 1')
    + '\nL14:\n',
    op('getlocal0') + op('getlocal1') + op('callproperty', f'{q("hbxState")}, 1')
    + op('iffalse', 'LHBXASK') + '\n'
    + op('getlex', WS) + op('getlocal1') + op('getproperty', q('ItemPath'))
    + op('callpropvoid', f'{q("LoadWorkshopMap")}, 1')
    + op('jump', 'L14') + '\n'
    + 'LHBXASK:\n'
    # a new tag: older movies log HBXHOST on every host, after loading the map
    + log('HBXHOSTREQ', op('getlocal1'), op('getproperty', q('FullPath')))
    + PAD
    + 'L14:\n',
)

# 2 a plain pick that hosts goes through hbxHost too
rep(
    'L14:\n' + op('getlex', WS) + op('getlocal2') + op('getproperty', q('Filename'))
    + op('pushstring', '"h"') + op('callproperty', f'{INDEX_OF}, 1')
    + op('pushbyte', '0') + op('ifnge', 'L25') + '\n'
    + op('getlocal0') + op('getlocal2') + op('callproperty', f'{q("hbxPath")}, 1')
    + op('jump', 'L27') + '\n'
    + 'L25:\n',
    'L14:\n' + op('getlocal2') + op('getproperty', q('Filename'))
    + op('pushstring', '"h"') + op('callproperty', f'{INDEX_OF}, 1')
    + op('pushbyte', '0') + op('ifnge', 'L25') + '\n'
    + op('getlocal0') + op('getlocal2') + op('callpropvoid', f'{q("hbxHost")}, 1')
    + op('jump', 'L28') + '\n'
    + 'L25:\n' + op('getlex', WS),
)

open(path, 'w', encoding='utf-8').write(s)
