import shutil, os, subprocess
R = os.path.expanduser('~/Documents/hebnix-rl-re/tools/rabc')
M = 'menus:ServerBrowserMenu'
def q(n): return f'QName(PackageNamespace(""), "{n}")'
def fq(p, n): return f'QName(PackageNamespace("{p}"), "{n}")'
LEGX = 'QName(ProtectedNamespace("tagame.menus:TabMenu"), "m_buttonLegend")'
CTRL = fq("tagame.constants", "Controller")
SIM = fq('projectx.managers', 'SystemIconManager')
DEV = fq("tagame.api", "DevUtil")
BLD = fq("tagame.data", "ButtonLegendData")

def ops(text):
    out = ''
    for l in text.strip().splitlines():
        l = l.strip()
        if not l: continue
        out += (l + '\n') if l.endswith(':') else '      ' + l + '\n'
    return out

def method(name, params, body, stack, locs, ret='void'):
    ps = ''.join(f'    param {p}\n' for p in params)
    r = q(ret) if ret not in (None, '*') else 'null'
    tail = '      returnvoid\n' if ret == 'void' else ''
    return f'''  trait method {q(name)}
   method
    refid "{M}/instance/{name}"
{ps}    returns {r}
    body
     maxstack {stack}
     localcount {locs}
     initscopedepth 14
     maxscopedepth 15
     code
      getlocal0
      pushscope

{body}{tail}     end ; code
    end ; body
   end ; method
  end ; trait
'''

def log(tag, expr):
    return f'''getlex {DEV}
pushstring "{tag}"
{expr}
callpropvoid {q("Log")}, 2
'''

# the selected server row goes to Hebnix as owner|server|map
sync = ops(f'''
getlocal1
iffalse LEND
''' + log('HBXSYNC', f'''getlocal1
getproperty {q("OwnerName")}
pushstring "|"
add
getlocal1
getproperty {q("ServerName")}
add
pushstring "|"
add
getlocal1
getproperty {q("LocalizedMapName")}
add''') + ''.join(log('HBXPAD', 'pushstring "' + ' ' * 900 + '"') for _ in range(6)) + '''
LEND:
''')
syncsel = ops(f'''
getlocal0
getproperty {q("list")}
getproperty {q("dataProvider")}
iffalse LEND
getlocal0
getproperty {q("list")}
getproperty {q("dataProvider")}
getlocal0
getproperty {q("list")}
getproperty {q("selectedIndex")}
callproperty {q("requestItemAt")}, 1
setlocal1
getlocal0
getlocal1
callpropvoid {q("hbxSync")}, 1
LEND:
''')
right = ops(f'''
getlocal0
getlocal1
getproperty {q("itemData")}
callpropvoid {q("hbxSync")}, 1
''')
synckey = ops(f'''
getlocal0
callpropvoid {q("hbxSyncSel")}, 0
''')
legdata = ops(f'''
getlex {fq("projectx.api", "ShellAPI")}
getproperty {q("inputType")}
getlex {fq("projectx.api", "InputTypes")}
getproperty {q("PC")}
equals
iffalse LPAD
findpropstrict {BLD}
pushstring "RMouse  SYNC MAP"
pushstring "RMouse"
pushstring "HbxSync"
constructprop {BLD}, 3
returnvalue
LPAD:
findpropstrict {BLD}
pushstring "SYNC MAP"
getlex {CTRL}
getproperty {q("Y")}
pushstring "HbxSync"
constructprop {BLD}, 3
returnvalue
''')
icon = ops(f'''
getlex {fq("projectx.api", "ShellAPI")}
getproperty {q("inputType")}
getlex {fq("projectx.api", "InputTypes")}
getproperty {q("PC")}
ifne LEND
getlex {LEGX}
iffalse LEND
getlex {LEGX}
getlex {LEGX}
getproperty {q("numElements")}
pushbyte 1
subtract
callproperty {q("getElement")}, 1
setlocal1
getlocal1
iffalse LEND
pushbyte 0
setlocal2
jump LCOND
LBODY:
label
getlocal1
getlocal2
callproperty {q("getChildAt")}, 1
setlocal3
getlocal3
istype {fq("scaleform.clik.core", "UIComponent")}
iffalse LNEXT
getlocal3
getproperty {q("textField")}
setlocal 4
getlocal 4
iffalse LNEXT
getlex {SIM}
getlocal 4
pushdouble 1.4
pushbyte -2
callpropvoid {q("replaceMarkupWithIcons")}, 3
LNEXT:
inclocal_i 2
LCOND:
getlocal2
getlocal1
getproperty {q("numChildren")}
iflt LBODY
LEND:
''')
relegend = ops(f'''
getlex {LEGX}
iffalse LEND
getlex {LEGX}
getproperty {q("numElements")}
pushbyte 4
iflt LEND
getlex {LEGX}
pushbyte 3
pushbyte 1
callpropvoid {q("removeElements")}, 2
getlex {LEGX}
getlocal0
callproperty {q("hbxLegendData")}, 0
newarray 1
callpropvoid {q("addElements")}, 1
getlocal0
callpropvoid {q("hbxIcon")}, 0
LEND:
''')
methods = (method('hbxSync', ['null'], sync, 6, 2)
    + method('hbxSyncSel', [], syncsel, 4, 2)
    + method('hbxRight', [fq('scaleform.clik.events', 'ListEvent')], right, 3, 2)
    + method('hbxSyncKey', ['null'], synckey, 2, 2)
    + method('hbxLegendData', [], legdata, 5, 1, ret='*')
    + method('hbxIcon', [], icon, 6, 5)
    + method('hbxRelegend', [], relegend, 5, 1))

PB = lambda s: s
edits = [
 # Y on a controller
 ('      setlocal3\n\n      jump                L31\n',
  '      setlocal3\n\n      getlocal2\n      getproperty         ' + q("navEquivalent") + '\n      getlex              ' + fq("scaleform.clik.constants", "NavigationCode") + '\n      getproperty         ' + q("GAMEPAD_Y") + '\n      ifstrictne          LHBXNOY\n\n      getlocal3\n      iffalse             LHBXNOY\n\n      getlocal0\n      callpropvoid        ' + q("hbxSyncSel") + ', 0\n\n      getlocal1\n      pushtrue\n      setproperty         ' + q("handled") + '\n\nLHBXNOY:\n      jump                L31\n'),
 # legend event + right click, added and removed with the menu
 ('      getproperty         QName(ProtectedNamespace("menus:ServerBrowserMenu"), "handleRefreshList")\n      callpropvoid        QName(PackageNamespace(""), "addEventListener"), 2\n\nL47:',
  '      getproperty         QName(ProtectedNamespace("menus:ServerBrowserMenu"), "handleRefreshList")\n      callpropvoid        QName(PackageNamespace(""), "addEventListener"), 2\n\n      getlex              ' + LEGX + '\n      pushstring          "HbxSync"\n      getlocal0\n      getproperty         ' + q("hbxSyncKey") + '\n      callpropvoid        ' + q("addEventListener") + ', 2\n\nL47:'),
 ('"handleListItemClick")\n      callpropvoid        QName(PackageNamespace(""), "addEventListener"), 2\n\n      getlocal0\n      callpropvoid        QName(ProtectedNamespace("menus:ServerBrowserMenu"), "updateButtonLegend"), 0',
  '"handleListItemClick")\n      callpropvoid        QName(PackageNamespace(""), "addEventListener"), 2\n\n      getlocal0\n      getproperty         ' + q("list") + '\n      getlex              ' + fq("scaleform.clik.events", "ListEvent") + '\n      getproperty         ' + q("ITEM_RIGHT_CLICK") + '\n      getlocal0\n      getproperty         ' + q("hbxRight") + '\n      callpropvoid        ' + q("addEventListener") + ', 2\n\n      getlocal0\n      callpropvoid        QName(ProtectedNamespace("menus:ServerBrowserMenu"), "updateButtonLegend"), 0'),
 ('      getproperty         QName(ProtectedNamespace("menus:ServerBrowserMenu"), "handleRefreshList")\n      callpropvoid        QName(PackageNamespace(""), "removeEventListener"), 2\n\nL33:',
  '      getproperty         QName(ProtectedNamespace("menus:ServerBrowserMenu"), "handleRefreshList")\n      callpropvoid        QName(PackageNamespace(""), "removeEventListener"), 2\n\n      getlex              ' + LEGX + '\n      pushstring          "HbxSync"\n      getlocal0\n      getproperty         ' + q("hbxSyncKey") + '\n      callpropvoid        ' + q("removeEventListener") + ', 2\n\nL33:'),
 ('"handleListItemClick")\n      callpropvoid        QName(PackageNamespace(""), "removeEventListener"), 2\n\n      returnvoid',
  '"handleListItemClick")\n      callpropvoid        QName(PackageNamespace(""), "removeEventListener"), 2\n\n      getlocal0\n      getproperty         ' + q("list") + '\n      getlex              ' + fq("scaleform.clik.events", "ListEvent") + '\n      getproperty         ' + q("ITEM_RIGHT_CLICK") + '\n      getlocal0\n      getproperty         ' + q("hbxRight") + '\n      callpropvoid        ' + q("removeEventListener") + ', 2\n\n      returnvoid'),
 # the sync entry comes and goes with the join entry
 ('      getlocal0\n      getproperty         QName(ProtectedNamespace("menus:ServerBrowserMenu"), "m_joinMatchLegendData")\n      newarray            1\n      callpropvoid        QName(PackageNamespace(""), "addElements"), 1\n',
  '      getlocal0\n      getproperty         QName(ProtectedNamespace("menus:ServerBrowserMenu"), "m_joinMatchLegendData")\n      getlocal0\n      callproperty        ' + q("hbxLegendData") + ', 0\n      newarray            2\n      callpropvoid        QName(PackageNamespace(""), "addElements"), 1\n\n      getlocal0\n      callpropvoid        ' + q("hbxIcon") + ', 0\n'),
 ('      pushbyte            2\n      pushbyte            1\n      callpropvoid        QName(PackageNamespace(""), "removeElements"), 2',
  '      pushbyte            2\n      pushbyte            2\n      callpropvoid        QName(PackageNamespace(""), "removeElements"), 2'),
 ('     maxstack 3\n     localcount 1\n     initscopedepth 14\n     maxscopedepth 15\n     code\n      getlocal0\n      pushscope\n\n      getlex              QName(ProtectedNamespace("tagame.menus:TabMenu"), "m_buttonLegend")\n      iftrue              L5',
  '     maxstack 6\n     localcount 1\n     initscopedepth 14\n     maxscopedepth 15\n     code\n      getlocal0\n      pushscope\n\n      getlex              QName(ProtectedNamespace("tagame.menus:TabMenu"), "m_buttonLegend")\n      iftrue              L5'),
 # keyboard <-> controller switch
 ('callsupervoid       QName(ProtectedNamespace("tagame.menus:TabMenu"), "handleInputTypeChanged"), 1\nL15:\n      returnvoid\n',
  'callsupervoid       QName(ProtectedNamespace("tagame.menus:TabMenu"), "handleInputTypeChanged"), 1\nL15:\n      getlocal0\n      callpropvoid        ' + q("hbxRelegend") + ', 0\n      returnvoid\n'),
 (' end ; instance\n', methods + ' end ; instance\n'),
]

def build(name):
    shutil.rmtree(name, ignore_errors=True)
    os.makedirs(name)
    shutil.copytree('in-0', f'{name}/in-0')
    f = f'{name}/in-0/menus/ServerBrowserMenu.class.asasm'
    s = open(f, encoding='utf-8').read()
    for old, new in edits:
        assert s.count(old) == 1, (s.count(old), old[:80])
        s = s.replace(old, new)
    open(f, 'w', encoding='utf-8').write(s)
    shutil.copy('in.swf', f'{name}/o.swf')
    subprocess.run([R + '/rabcasm.exe', f'{name}/in-0/in-0.main.asasm'], check=True)
    subprocess.run([R + '/abcreplace.exe', f'{name}/o.swf', '0', f'{name}/in-0/in-0.main.abc'], check=True)

build('s1')
print('built s1')
