# Instant Suite winner movie: when the winner is announced (or at the podium) ask Hebnix (preview server, /is/) what to do.
# The PNG's width is the mode: 1 requeue, 2 freeplay, 3 main menu, 4 workshop map, else nothing.
import shutil, os, subprocess, sys
R = os.path.expanduser('~/Documents/hebnix-rl-re/tools/rabc')
M = 'prompts:WinnerPrompt'
def q(n): return f'QName(PackageNamespace(""), "{n}")'
def fq(p, n): return f'QName(PackageNamespace("{p}"), "{n}")'
def pq(n): return f'QName(ProtectedNamespace("{M}"), "{n}")'
BI = 'QName(Namespace("http://adobe.com/AS3/2006/builtin"), "indexOf")'
HOOKS = fq('projectx.proxy', 'UIShellHooks')
DATA = fq('projectx.api', 'DataAPI')
URL = 'img://http://127.0.0.1:47853/is/'

def ops(text):
    out = ''
    for l in text.strip().splitlines():
        l = l.strip()
        if l:
            out += (l + '\n') if l.endswith(':') else '      ' + l + '\n'
    return out

def method(name, params, body, stack, locs):
    ps = ''.join(f'    param {p}\n' for p in params)
    return f'''  trait method {q(name)}
   method
    refid "{M}/instance/{name}"
{ps}    returns {q("void")}
    body
     maxstack {stack}
     localcount {locs}
     initscopedepth 12
     maxscopedepth 13
     code
      getlocal0
      pushscope

{ops(body)}      returnvoid
     end ; code
    end ; body
   end ; method
  end ; trait
'''

# once per match: the winner banner and the podium both call this
ASK = f'''
getlocal0
getproperty {q("hbxDone")}
iffalse LHBXGO
returnvoid
LHBXGO:
getlocal0
pushtrue
setproperty {q("hbxDone")}
findpropstrict {fq("flash.display", "Loader")}
constructprop {fq("flash.display", "Loader")}, 0
setlocal1
getlocal0
getlocal1
setproperty {q("hbxL")}
getlocal1
getproperty {q("contentLoaderInfo")}
pushstring "complete"
getlocal0
getproperty {q("hbxGot")}
callpropvoid {q("addEventListener")}, 2
getlocal1
getproperty {q("contentLoaderInfo")}
pushstring "ioError"
getlocal0
getproperty {q("hbxNone")}
callpropvoid {q("addEventListener")}, 2
getlocal1
findpropstrict {fq("flash.net", "URLRequest")}
pushstring "{URL}"
findpropstrict {q("Date")}
constructprop {q("Date")}, 0
getproperty {q("time")}
add
pushstring ".png"
add
constructprop {fq("flash.net", "URLRequest")}, 1
callpropvoid {q("load")}, 1
'''

GOT = f'''
getlocal1
getproperty {q("target")}
getproperty {q("width")}
convert_i
setlocal2
getlocal2
pushbyte 1
ifne LHBX2
getlocal0
callpropvoid {pq("onQuickRequeue")}, 0
returnvoid
LHBX2:
getlocal2
pushbyte 2
ifne LHBX3
getlocal0
callpropvoid {pq("handleFreeplayRequest")}, 0
returnvoid
LHBX3:
getlocal2
pushbyte 3
ifne LHBX4
getlex {fq("tagame.api", "Shell")}
callpropvoid {q("ExitToMainMenu")}, 0
returnvoid
LHBX4:
getlocal2
pushbyte 4
ifne LHBXEND
getlex {HOOKS}
getproperty {q("SHELL_CallUnrealFunction")}
pushnull
ifeq LHBXEND
getlex {HOOKS}
pushstring "GFxData_Workshop_TA"
pushstring "Workshop"
callpropvoid {q("SHELL_RegisterSingleton")}, 2
getlex {DATA}
getlocal0
getproperty {q("hbxTable")}
pushstring "WorkshopItems"
callpropvoid {q("registerOnTableChanged")}, 2
getlex {HOOKS}
pushstring "GFxData_Workshop_TA"
pushstring "Workshop"
pushbyte 0
pushstring "LoadAllWorkshopMapData"
callpropvoid {q("SHELL_CallUnrealFunction")}, 4
LHBXEND:
'''

# the list Hebnix writes flags the map to load with "n" in Filename
TABLE = f'''
getlex {DATA}
getlocal0
getproperty {q("hbxTable")}
callpropvoid {q("unregisterCallback")}, 1
getlex {DATA}
pushstring "WorkshopItems"
callproperty {q("getRowCount")}, 1
convert_i
setlocal1
pushbyte 0
setlocal2
jump LHBXCHK
LHBXLOOP:
label
getlex {DATA}
pushstring "WorkshopItems"
getlocal2
pushstring "Filename"
callproperty {q("getValue")}, 3
convert_s
pushstring "n"
callproperty {BI}, 1
pushbyte 0
ifnge LHBXNEXT
getlex {HOOKS}
pushstring "GFxData_Workshop_TA"
pushstring "Workshop"
pushbyte 0
pushstring "LoadWorkshopMap"
getlex {DATA}
pushstring "WorkshopItems"
getlocal2
pushstring "FullPath"
callproperty {q("getValue")}, 3
convert_s
callpropvoid {q("SHELL_CallUnrealFunction")}, 5
returnvoid
LHBXNEXT:
inclocal_i 2
LHBXCHK:
getlocal2
getlocal1
iflt LHBXLOOP
'''

METHODS = (method('hbxAsk', [], ASK, 8, 2)
           + method('hbxGot', ['null'], GOT, 8, 3)
           + method('hbxNone', ['null'], '', 1, 2)
           + method('hbxTable', [], TABLE, 12, 3))
SLOTS = f'  trait slot {q("hbxL")} type null end\n  trait slot {q("hbxDone")} type {q("Boolean")} end\n'

def build(name):
    shutil.rmtree(name, ignore_errors=True)
    shutil.copytree('rt', f'{name}/rt')
    f = f'{name}/rt/prompts/WinnerPrompt.class.asasm'
    s = open(f, encoding='utf-8').read()
    start = s.index(f'trait method {pq("showPodiumSpotlight")}')
    end = s.index('      returnvoid\n     end ; code', start)
    ask = f'      getlocal0\n      callpropvoid        {q("hbxAsk")}, 0\n\n'
    s = s[:end] + ask + s[end:]
    # the winner banner: onWinningTeamChanged with a winner shows the prompt mid-screen
    start = s.index(f'trait method {pq("onWinningTeamChanged")}')
    reg = 'callpropvoid        QName(PackageNamespace(""), "registerOnGameStateChanged"), 1\n'
    at = s.index(reg, start) + len(reg)
    s = s[:at] + '\n' + ask + s[at:]
    assert s.count(' end ; instance\n') == 1
    s = s.replace(' end ; instance\n', SLOTS + METHODS + ' end ; instance\n')
    open(f, 'w', encoding='utf-8').write(s)
    shutil.copy('in.swf', f'{name}/o.swf')
    subprocess.run([R + '/rabcasm.exe', f'{name}/rt/in-0.main.asasm'], check=True)
    subprocess.run([R + '/abcreplace.exe', f'{name}/o.swf', '0', f'{name}/rt/in-0.main.abc'], check=True)

build(sys.argv[1] if len(sys.argv) > 1 else 'out')
