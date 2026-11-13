exec(open('mk2.py').read())
def q(n): return f'QName(PackageNamespace(""), "{n}")'
def fq(p,n): return f'QName(PackageNamespace("{p}"), "{n}")'
DYN='MultinameL([PackageNamespace("")])'
LIST=q('workshopItemsList')
LEG='QName(PrivateNamespace(null, "menus:WorkshopMenu"), "_buttonLegend")'
M='menus:WorkshopMenu'
CTRL=fq("tagame.constants","Controller")
SIM=fq('projectx.managers','SystemIconManager')
SPRITE=fq("flash.display","Sprite")
DEV=fq("tagame.api","DevUtil")
BOXES=5
def BI(n): return f'QName(Namespace("http://adobe.com/AS3/2006/builtin"), "{n}")'

def ops(text):
    out=''
    for l in text.strip().splitlines():
        l=l.strip()
        if not l: continue
        out+= (l+'\n') if l.endswith(':') else '      '+l+'\n'
    return out

def method(name,params,body,stack,locs,ret='void'):
    ps=''.join(f'    param {p}\n' for p in params)
    r = q(ret) if ret not in (None,'*') else 'null'
    tail = '      returnvoid\n' if ret=='void' else ''
    return f'''  trait method {q(name)}
   method
    refid "{M}/instance/{name}"
{ps}    returns {r}
    body
     maxstack {stack}
     localcount {locs}
     initscopedepth 13
     maxscopedepth 14
     code
      getlocal0
      pushscope

{body}{tail}     end ; code
    end ; body
   end ; method
  end ; trait
'''

def log(tag,expr):
    return f'''getlex {DEV}
pushstring "{tag}"
{expr}
callpropvoid {q("Log")}, 2
'''

state=ops(f'''
getlocal0
getproperty {q("hbxTicks")}
getlocal1
getproperty {q("FullPath")}
getproperty {DYN}
setlocal2
getlocal2
pushundefined
ifstrictne LHAVE
getlocal1
getproperty {q("Filename")}
pushstring "t"
callproperty {BI("indexOf")}, 1
pushbyte 0
greaterequals
returnvalue
LHAVE:
getlocal2
convert_b
returnvalue
''')
toggle=ops(f'''
jump LEND
getlocal1
iffalse LEND
getlocal1
getproperty {q("FullPath")}
setlocal2
getlocal0
getlocal1
callproperty {q("hbxState")}, 1
not
setlocal3
getlocal0
getproperty {q("hbxTicks")}
getlocal2
getlocal3
setproperty {DYN}
''' + log('HBXTICK', f'getlocal3\npushstring " "\nadd\ngetlocal2\nadd') + ''.join(log('HBXPAD','pushstring "'+' '*900+'"') for _ in range(6)) + '''
LEND:
''')
path=ops(f'''
getlocal0
getlocal1
callproperty {q("hbxState")}, 1
iffalse LREAL
getlocal1
getproperty {q("ItemPath")}
returnvalue
LREAL:
getlocal1
getproperty {q("FullPath")}
pushstring "?HbxHost.txt"
add
returnvalue
''')
host=ops(f'''
getlocal1
iffalse LEND
''' + log('HBXHOST', f'getlocal1\ngetproperty {q("FullPath")}') + f'''
getlex {fq("tagame.api","Workshop")}
getlocal0
getlocal1
callproperty {q("hbxPath")}, 1
callpropvoid {q("LoadWorkshopMap")}, 1
LEND:
''')
selected=f'''getlocal0
getproperty {LIST}
getproperty {q("dataProvider")}
getlocal0
getproperty {LIST}
getproperty {q("selectedIndex")}
callproperty {q("requestItemAt")}, 1
'''
draw=ops(f'''
getlocal1
getproperty {q("graphics")}
setlocal3
getlocal3
callpropvoid {q("clear")}, 0
getlocal3
pushint 399142
pushbyte 1
callpropvoid {q("beginFill")}, 2
getlocal3
pushbyte 2
pushint 8180735
callpropvoid {q("lineStyle")}, 2
getlocal3
pushbyte 0
pushbyte 0
pushbyte 22
pushbyte 22
callpropvoid {q("drawRect")}, 4
getlocal3
callpropvoid {q("endFill")}, 0
getlocal2
iffalse LOFF
getlocal3
pushbyte 3
pushint 16777215
callpropvoid {q("lineStyle")}, 2
getlocal3
pushbyte 4
pushbyte 11
callpropvoid {q("moveTo")}, 2
getlocal3
pushbyte 9
pushbyte 16
callpropvoid {q("lineTo")}, 2
getlocal3
pushbyte 18
pushbyte 4
callpropvoid {q("lineTo")}, 2
LOFF:
''')
init=ops(f'''
getlocal0
getproperty {q("hbxBoxes")}
iftrue LEND
getlocal0
newarray 0
setproperty {q("hbxBoxes")}
getlocal0
newarray 0
setproperty {q("hbxShown")}
getlocal0
newobject 0
setproperty {q("hbxTicks")}
pushbyte 0
setlocal1
jump LCOND
LBODY:
label
findpropstrict {fq("flash.utils","getDefinitionByName")}
pushstring "OptionsCheckBoxMovie"
callproperty {fq("flash.utils","getDefinitionByName")}, 1
construct 0
setlocal2
getlocal2
getlex {fq("scaleform.clik.events","ButtonEvent")}
getproperty {q("CLICK")}
getlocal0
getproperty {q("hbxBoxClick")}
callpropvoid {q("addEventListener")}, 2
getlocal0
getlocal2
callpropvoid {q("addChild")}, 1
getlocal0
getproperty {q("hbxBoxes")}
getlocal2
callpropvoid {BI("push")}, 1
inclocal_i 1
LCOND:
getlocal1
pushbyte {BOXES}
iflt LBODY
getlocal0
pushstring "enterFrame"
getlocal0
getproperty {q("hbxUpdate")}
callpropvoid {q("addEventListener")}, 2
LEND:
''')
update=ops(f'''
getlex {fq("projectx.core","UIMenu")}
callproperty {q("getActiveMenu")}, 0
getlocal0
ifstrictne LOTHER
getlocal0
getproperty {q("hbxMode")}
pushbyte 0
ifne LLEGDONE
getlocal0
callpropvoid {q("hbxLegendOn")}, 0
jump LLEGDONE
LOTHER:
getlocal0
pushbyte 0
setproperty {q("hbxMode")}
LLEGDONE:
pushbyte 0
setlocal2
jump LCOND
LBODY:
label
getlocal0
getproperty {LIST}
getproperty {q("scrollPosition")}
getlocal2
add
convert_i
setlocal 5
getlocal0
getproperty {q("hbxBoxes")}
getlocal2
getproperty {DYN}
setlocal 4
pushnull
setlocal3
getlocal2
getlocal0
getproperty {LIST}
getproperty {q("rowCount")}
ifge LNOROW
getlocal 5
getlocal0
getproperty {LIST}
getproperty {q("dataProvider")}
getproperty {q("length")}
ifge LNOROW
getlocal0
getproperty {LIST}
getproperty {q("dataProvider")}
getlocal 5
callproperty {q("requestItemAt")}, 1
setlocal3
LNOROW:
getlocal0
getproperty {q("hbxDl")}
iffalse LNODL
pushnull
setlocal3
LNODL:
getlocal 4
pushfalse
setproperty {q("visible")}
getlocal3
iffalse LNEXT
getlocal0
getproperty {LIST}
getlocal2
callproperty {q("getRendererAt")}, 1
setlocal 7
getlocal 7
iffalse LNEXT
getlocal 4
getlocal0
getproperty {LIST}
getproperty {q("x")}
getlocal0
getproperty {LIST}
getproperty {q("width")}
add
pushbyte 10
add
setproperty {q("x")}
getlocal 4
getlocal0
getproperty {LIST}
getproperty {q("y")}
getlocal 7
getproperty {q("y")}
add
getlocal 7
getproperty {q("height")}
getlocal 4
getproperty {q("height")}
subtract
pushbyte 2
divide
add
setproperty {q("y")}
getlocal0
getlocal3
callproperty {q("hbxState")}, 1
convert_b
setlocal 6
getlocal 4
getproperty {q("selected")}
getlocal 6
ifstricteq LSELOK
getlocal 4
getlocal 6
setproperty {q("selected")}
LSELOK:
getlocal0
getproperty {q("hbxOnBox")}
iffalse LNOFOC
getlocal 5
getlocal0
getproperty {LIST}
getproperty {q("selectedIndex")}
ifne LNOFOC
pushbyte 1
jump LFOC
LNOFOC:
pushbyte 0
LFOC:
setlocal 8
getlocal 4
getproperty {q("focused")}
getlocal 8
ifeq LNEXT
getlocal 4
getlocal 8
setproperty {q("focused")}
LNEXT:
inclocal_i 2
LCOND:
getlocal2
pushbyte {BOXES}
iflt LBODY
''')
click=ops(f'''
getlocal0
getproperty {q("hbxBoxes")}
getlocal1
getproperty {q("currentTarget")}
callproperty {BI("indexOf")}, 1
convert_i
setlocal2
getlocal0
getlocal0
getproperty {LIST}
getproperty {q("dataProvider")}
getlocal0
getproperty {LIST}
getproperty {q("scrollPosition")}
getlocal2
add
callproperty {q("requestItemAt")}, 1
callpropvoid {q("hbxToggleRow")}, 1
''')
layout=ops(f'''
getlocal0
getproperty {LIST}
getproperty {q("dataProvider")}
getproperty {q("length")}
pushbyte 0
ifngt LEND
getlocal0
getproperty {q("background")}
getlocal0
getproperty {q("background")}
getproperty {q("width")}
pushbyte 76
add
setproperty {q("width")}
getlocal0
getproperty {q("background")}
getlocal0
getproperty {q("background")}
getproperty {q("width")}
pushdouble -0.5
multiply
setproperty {q("x")}
getlocal0
getproperty {q("closeButton")}
getlocal0
getproperty {q("background")}
getproperty {q("width")}
pushdouble 0.5
multiply
pushbyte 50
subtract
setproperty {q("x")}
getlocal0
getproperty {LIST}
getlocal0
getproperty {q("background")}
getproperty {q("x")}
pushbyte 9
add
setproperty {q("x")}
getlocal0
getproperty {q("scrollBar")}
getlocal0
getproperty {LIST}
getproperty {q("x")}
getlocal0
getproperty {LIST}
getproperty {q("width")}
add
pushbyte 56
add
setproperty {q("x")}
getlocal0
callpropvoid {q("hbxMake")}, 0
LEND:
''')
INPUT=ops(f'''
getlocal1
getproperty {q("details")}
setlocal2
getlocal2
getproperty {q("navEquivalent")}
setlocal3
getlocal2
getproperty {q("value")}
setlocal 4
getlocal0
getproperty {q("hbxDl")}
iffalse LNOTG
getlocal0
getlocal3
getlocal 4
getlocal2
getproperty {q("code")}
callpropvoid {q("hbxGridKey")}, 3
getlocal1
pushtrue
setproperty {q("handled")}
jump LEND
LNOTG:
getlocal3
getlex {fq("scaleform.clik.constants","NavigationCode")}
getproperty {q("GAMEPAD_L3")}
ifne LNOTX
getlocal 4
getlex {fq("scaleform.clik.constants","InputValue")}
getproperty {q("KEY_UP")}
ifne LEND
getlocal1
pushtrue
setproperty {q("handled")}
getlocal0
callpropvoid {q("hbxToggleDl")}, 0
jump LEND
LNOTX:
getlocal3
getlex {fq("scaleform.clik.constants","NavigationCode")}
getproperty {q("GAMEPAD_Y")}
ifne LNOTY
getlocal 4
getlex {fq("scaleform.clik.constants","InputValue")}
getproperty {q("KEY_UP")}
ifne LEND
getlocal1
pushtrue
setproperty {q("handled")}
getlocal0
''' + selected + f'''
callpropvoid {q("hbxHost")}, 1
jump LEND
LNOTY:
getlocal3
getlex {fq("scaleform.clik.constants","NavigationCode")}
getproperty {q("RIGHT")}
ifne LNOTR
getlocal 4
getlex {fq("scaleform.clik.constants","InputValue")}
getproperty {q("KEY_DOWN")}
ifne LEND
getlocal0
pushtrue
setproperty {q("hbxOnBox")}
jump LEND
LNOTR:
getlocal3
getlex {fq("scaleform.clik.constants","NavigationCode")}
getproperty {q("LEFT")}
ifne LNOTL
getlocal 4
getlex {fq("scaleform.clik.constants","InputValue")}
getproperty {q("KEY_DOWN")}
ifne LEND
getlocal0
pushfalse
setproperty {q("hbxOnBox")}
jump LEND
LNOTL:
getlocal0
getproperty {q("hbxOnBox")}
iffalse LEND
getlocal3
getlex {fq("scaleform.clik.constants","NavigationCode")}
getproperty {q("ENTER")}
ifeq LISA
getlocal3
getlex {fq("scaleform.clik.constants","NavigationCode")}
getproperty {q("GAMEPAD_A")}
ifne LEND
LISA:
getlocal1
pushtrue
setproperty {q("handled")}
getlocal 4
getlex {fq("scaleform.clik.constants","InputValue")}
getproperty {q("KEY_UP")}
ifne LEND
getlocal0
''' + selected + f'''
callpropvoid {q("hbxToggleRow")}, 1
LEND:
''')
right=ops(f'''
getlocal0
getlocal1
getproperty {q("itemData")}
callpropvoid {q("hbxHost")}, 1
''')
def data(label, markup, event):
    return f'''findpropstrict {fq("tagame.data","ButtonLegendData")}
pushstring "{label}"
{markup}
pushstring "{event}"
constructprop {fq("tagame.data","ButtonLegendData")}, 3
'''
PAD_HOST=f'getlex {CTRL}\ngetproperty {q("Y")}'
PAD_TICK=f'getlex {CTRL}\ngetproperty {q("Menu")}'
icon=ops(f'''
getlocal0
getproperty {LEG}
getlocal0
getproperty {LEG}
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
istype {fq("scaleform.clik.core","UIComponent")}
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
hostkey=ops(f'''
getlocal0
''' + selected + f'''
callpropvoid {q("hbxHost")}, 1
''')
legon=ops(f'''
getlocal0
getproperty {LEG}
iffalse LEND
getlocal0
callproperty {q("hbxCountDl")}, 0
pushbyte 0
ifngt LNODL
getlex {fq("projectx.api","ShellAPI")}
getproperty {q("inputType")}
getlex {fq("projectx.api","InputTypes")}
getproperty {q("PC")}
equals
iffalse LDLPAD
pushstring "HbxDl"
setlocal1
jump LDLGO
LDLPAD:
getlex {CTRL}
getproperty {q("LThumb")}
setlocal1
LDLGO:
getlocal0
pushtrue
setproperty {q("hbxLegDl")}
getlocal0
getproperty {LEG}
''' + data('DOWNLOAD MAPS','getlocal1','HbxDl') + f'''
newarray 1
callpropvoid {q("addElements")}, 1
getlocal0
getproperty {LEG}
pushstring "HbxDl"
getlocal0
getproperty {q("hbxDlKey")}
callpropvoid {q("addEventListener")}, 2
LNODL:
getlex {fq("projectx.api","ShellAPI")}
getproperty {q("inputType")}
getlex {fq("projectx.api","InputTypes")}
getproperty {q("PC")}
equals
iffalse LPAD
getlocal0
pushbyte 1
setproperty {q("hbxMode")}
getlocal0
getproperty {LEG}
''' + data('RMouse  HOST LAN','pushstring "RMouse"','HbxHost') + f'''
jump LADD
LPAD:
getlocal0
pushbyte 2
setproperty {q("hbxMode")}
getlocal0
getproperty {LEG}
''' + data('HOST LAN',PAD_HOST,'HbxHost') + f'''
LADD:
newarray 1
callpropvoid {q("addElements")}, 1
getlocal0
getproperty {LEG}
pushstring "HbxHost"
getlocal0
getproperty {q("hbxHostKey")}
callpropvoid {q("addEventListener")}, 2
getlocal0
getproperty {q("hbxMode")}
pushbyte 1
ifne LEND
getlocal0
callpropvoid {q("hbxIcon")}, 0
LEND:
''')
legoff=ops(f'''
getlocal0
getproperty {LEG}
iffalse LEND
getlocal0
getproperty {q("hbxLegDl")}
iffalse LNODL
pushnull
setlocal1
getlocal0
getproperty {q("hbxMode")}
pushbyte 1
ifne LDLA
pushstring "HbxDl"
setlocal1
LDLA:
getlocal0
getproperty {q("hbxMode")}
pushbyte 2
ifne LDLB
getlex {CTRL}
getproperty {q("LThumb")}
setlocal1
LDLB:
getlocal0
getproperty {LEG}
pushstring "HbxDl"
getlocal0
getproperty {q("hbxDlKey")}
callpropvoid {q("removeEventListener")}, 2
getlocal1
iffalse LDLC
getlocal0
getproperty {LEG}
getlocal1
callpropvoid {q("removeElementByGamepadMarkup")}, 1
LDLC:
getlocal0
pushfalse
setproperty {q("hbxLegDl")}
LNODL:
getlocal0
getproperty {LEG}
pushstring "HbxHost"
getlocal0
getproperty {q("hbxHostKey")}
callpropvoid {q("removeEventListener")}, 2
getlocal0
getproperty {q("hbxMode")}
pushbyte 1
ifne LNOTPC
getlocal0
getproperty {LEG}
pushstring "RMouse"
callpropvoid {q("removeElementByGamepadMarkup")}, 1
jump LDONE
LNOTPC:
getlocal0
getproperty {q("hbxMode")}
pushbyte 2
ifne LDONE
getlocal0
getproperty {LEG}
getlex {CTRL}
getproperty {q("Y")}
callpropvoid {q("removeElementByGamepadMarkup")}, 1
LDONE:
getlocal0
pushbyte 0
setproperty {q("hbxMode")}
LEND:
''')
inputchg=ops(f'''
getlocal0
getproperty {q("hbxMode")}
pushbyte 0
ifeq LEND
getlex {fq("projectx.core","UIMenu")}
callproperty {q("getActiveMenu")}, 0
getlocal0
ifstrictne LGONE
getlocal0
callpropvoid {q("hbxLegendOff")}, 0
getlocal0
callpropvoid {q("hbxLegendOn")}, 0
jump LEND
LGONE:
getlocal0
pushbyte 0
setproperty {q("hbxMode")}
LEND:
''')
make=ops(f'''
getlocal0
getproperty QName(PackageNamespace(""), "hbxSearch")
iftrue LPLACE
getlocal0
getproperty QName(PackageNamespace(""), "titleLabel")
getproperty QName(PackageNamespace(""), "textField")
getproperty QName(PackageNamespace(""), "defaultTextFormat")
setlocal2
getlex QName(PackageNamespace("flash.system"), "ApplicationDomain")
getproperty QName(PackageNamespace(""), "currentDomain")
pushstring "ModalSearchTextInput"
callproperty QName(PackageNamespace(""), "hasDefinition"), 1
iffalse LPLAIN
findpropstrict QName(PackageNamespace("flash.utils"), "getDefinitionByName")
pushstring "ModalSearchTextInput"
callproperty QName(PackageNamespace("flash.utils"), "getDefinitionByName"), 1
construct 0
setlocal1
getlocal1
pushstring "SEARCH"
setproperty QName(PackageNamespace(""), "keyboardTitleText")
getlocal1
pushstring ""
setproperty QName(PackageNamespace(""), "defaultText")
getlocal1
pushstring "change"
getlocal0
getproperty QName(PackageNamespace(""), "hbxFilter")
callpropvoid QName(PackageNamespace(""), "addEventListener"), 2
getlocal1
pushshort 190
pushbyte 42
callpropvoid QName(PackageNamespace(""), "setSize"), 2
jump LHAVE
LPLAIN:
findpropstrict QName(PackageNamespace("flash.text"), "TextField")
constructprop QName(PackageNamespace("flash.text"), "TextField"), 0
setlocal1
getlocal1
pushstring "input"
setproperty QName(PackageNamespace(""), "type")
getlocal1
pushtrue
setproperty QName(PackageNamespace(""), "border")
getlocal1
pushint 5017792
setproperty QName(PackageNamespace(""), "borderColor")
getlocal1
pushtrue
setproperty QName(PackageNamespace(""), "background")
getlocal1
pushint 660275
setproperty QName(PackageNamespace(""), "backgroundColor")
getlocal1
pushint 16777215
setproperty QName(PackageNamespace(""), "textColor")
getlocal1
pushshort 170
setproperty QName(PackageNamespace(""), "width")
getlocal1
pushbyte 26
setproperty QName(PackageNamespace(""), "height")
getlocal0
getproperty QName(PackageNamespace(""), "titleLabel")
getproperty QName(PackageNamespace(""), "textField")
getproperty QName(PackageNamespace(""), "defaultTextFormat")
setlocal2
getlocal2
pushbyte 20
setproperty QName(PackageNamespace(""), "size")
getlocal2
pushstring "left"
setproperty QName(PackageNamespace(""), "align")
getlocal1
getlocal2
setproperty QName(PackageNamespace(""), "defaultTextFormat")
getlocal1
pushstring "SEARCH"
setproperty QName(PackageNamespace(""), "text")
getlocal1
pushstring "change"
getlocal0
getproperty QName(PackageNamespace(""), "hbxFilter")
callpropvoid QName(PackageNamespace(""), "addEventListener"), 2
getlocal1
pushstring "focusIn"
getlocal0
getproperty QName(PackageNamespace(""), "hbxSearchFocus")
callpropvoid QName(PackageNamespace(""), "addEventListener"), 2
getlocal1
pushstring "focusOut"
getlocal0
getproperty QName(PackageNamespace(""), "hbxSearchBlur")
callpropvoid QName(PackageNamespace(""), "addEventListener"), 2
LHAVE:
getlocal0
getlocal1
setproperty QName(PackageNamespace(""), "hbxSearch")
getlocal0
getlocal1
callpropvoid QName(PackageNamespace(""), "addChild"), 1
findpropstrict QName(PackageNamespace("flash.text"), "TextField")
constructprop QName(PackageNamespace("flash.text"), "TextField"), 0
setlocal 3
getlocal 3
pushfalse
setproperty QName(PackageNamespace(""), "selectable")
getlocal 3
pushshort 90
setproperty QName(PackageNamespace(""), "width")
getlocal 3
pushbyte 24
setproperty QName(PackageNamespace(""), "height")
getlocal2
pushbyte 14
setproperty QName(PackageNamespace(""), "size")
getlocal 3
getlocal2
setproperty QName(PackageNamespace(""), "defaultTextFormat")
getlocal 3
pushint 12181503
setproperty QName(PackageNamespace(""), "textColor")
getlocal 3
pushstring ""
setproperty QName(PackageNamespace(""), "text")
getlocal0
getlocal 3
setproperty QName(PackageNamespace(""), "hbxLabel")
getlocal0
getlocal 3
callpropvoid QName(PackageNamespace(""), "addChild"), 1
LPLACE:
getlocal0
getproperty QName(PackageNamespace(""), "hbxLabel")
getlocal0
getproperty QName(PackageNamespace(""), "workshopItemsList")
getproperty QName(PackageNamespace(""), "x")
getlocal0
getproperty QName(PackageNamespace(""), "workshopItemsList")
getproperty QName(PackageNamespace(""), "width")
add
pushbyte 5
add
setproperty QName(PackageNamespace(""), "x")
getlocal0
getproperty QName(PackageNamespace(""), "hbxLabel")
getlocal0
getproperty QName(PackageNamespace(""), "workshopItemsList")
getproperty QName(PackageNamespace(""), "y")
pushbyte 26
subtract
setproperty QName(PackageNamespace(""), "y")
getlocal0
getproperty QName(PackageNamespace(""), "hbxSearch")
getlocal0
getproperty QName(PackageNamespace(""), "background")
getproperty QName(PackageNamespace(""), "x")
pushbyte 24
add
setproperty QName(PackageNamespace(""), "x")
getlocal0
getproperty QName(PackageNamespace(""), "hbxSearch")
getlocal0
getproperty QName(PackageNamespace(""), "titleLabel")
getproperty QName(PackageNamespace(""), "y")
pushbyte 16
add
setproperty QName(PackageNamespace(""), "y")
''')
filt=ops(f'''
getlocal0
getproperty {q("hbxDl")}
iffalse LNORM
getlocal0
callpropvoid {q("hbxGridRows")}, 0
jump LEND
LNORM:
getlocal0
getlex QName(PackageNamespace("tagame.api"), "WorkshopItems")
callproperty QName(PackageNamespace(""), "getAllRows"), 0
callproperty {q("hbxVisible")}, 1
coerce QName(PackageNamespace(""), "Array")
setlocal2
getlocal0
getproperty QName(PackageNamespace(""), "hbxSearch")
getproperty QName(PackageNamespace(""), "text")
callproperty QName(Namespace("http://adobe.com/AS3/2006/builtin"), "toLowerCase"), 0
setlocal3
newarray 0
setlocal 4
pushbyte 0
setlocal 5
jump LCOND
LBODY:
label
getlocal2
getlocal 5
getproperty MultinameL([PackageNamespace("")])
setlocal 6
getlocal3
getproperty QName(PackageNamespace(""), "length")
pushbyte 0
ifeq LKEEP
getlocal 6
getproperty QName(PackageNamespace(""), "Title")
convert_s
callproperty QName(Namespace("http://adobe.com/AS3/2006/builtin"), "toLowerCase"), 0
getlocal3
callproperty QName(Namespace("http://adobe.com/AS3/2006/builtin"), "indexOf"), 1
pushbyte 0
iflt LNEXT
LKEEP:
getlocal 4
getlocal 6
callpropvoid QName(Namespace("http://adobe.com/AS3/2006/builtin"), "push"), 1
LNEXT:
inclocal_i 5
LCOND:
getlocal 5
getlocal2
getproperty QName(PackageNamespace(""), "length")
iflt LBODY
getlocal0
getproperty QName(PackageNamespace(""), "workshopItemsList")
findpropstrict QName(PackageNamespace("scaleform.clik.data"), "DataProvider")
getlocal 4
constructprop QName(PackageNamespace("scaleform.clik.data"), "DataProvider"), 1
setproperty QName(PackageNamespace(""), "dataProvider")
getlocal0
getproperty QName(PackageNamespace(""), "loadingLabel")
getlocal 4
getproperty QName(PackageNamespace(""), "length")
pushbyte 0
equals
setproperty QName(PackageNamespace(""), "visible")
getlocal0
getproperty QName(PackageNamespace(""), "workshopItemsList")
getlex QName(PackageNamespace(""), "Math")
getlocal 4
getproperty QName(PackageNamespace(""), "length")
pushbyte 5
callproperty QName(PackageNamespace(""), "min"), 2
setproperty QName(PackageNamespace(""), "rowCount")
getlocal 4
getproperty QName(PackageNamespace(""), "length")
pushbyte 0
ifngt LEND
getlocal0
getproperty QName(PackageNamespace(""), "workshopItemsList")
pushbyte 0
setproperty QName(PackageNamespace(""), "selectedIndex")
LEND:
''')
focus=ops(f'''
getlocal0
getproperty QName(PackageNamespace(""), "hbxSearch")
getproperty QName(PackageNamespace(""), "text")
pushstring "SEARCH"
ifne LEND
getlocal0
getproperty QName(PackageNamespace(""), "hbxSearch")
pushstring ""
setproperty QName(PackageNamespace(""), "text")
LEND:
''')
blur=ops(f'''
getlocal0
getproperty QName(PackageNamespace(""), "hbxSearch")
getproperty QName(PackageNamespace(""), "text")
getproperty QName(PackageNamespace(""), "length")
pushbyte 0
ifne LEND
getlocal0
getproperty QName(PackageNamespace(""), "hbxSearch")
pushstring "SEARCH"
setproperty QName(PackageNamespace(""), "text")
LEND:
''')
def N(n):
    n = int(n)
    if -128 <= n <= 127: return f'pushbyte {n}'
    if -32768 <= n <= 32767: return f'pushshort {n}'
    return f'pushint {n}'
def J(*a): return '\n'.join(a)
def GP(o, n): return J(o, f'getproperty {q(n)}')
def SP(o, n, v): return J(o, v, f'setproperty {q(n)}')
def CV(o, n, *args): return J(o, *args, f'callpropvoid {q(n)}, {len(args)}')
def CP(o, n, *args): return J(o, *args, f'callproperty {q(n)}, {len(args)}')
def NAV(n): return J(f'getlex {fq("scaleform.clik.constants","NavigationCode")}', f'getproperty {q(n)}')
def IV(n): return J(f'getlex {fq("scaleform.clik.constants","InputValue")}', f'getproperty {q(n)}')
T = 'getlocal0'
UIS = fq('projectx.core', 'UIScreen')
TF = fq('flash.text', 'TextField')
IL = fq('projectx.controls', 'ImageLoader')
SHELL = fq('tagame.api', 'Shell')
WHITE = 'pushint 16777215'
BLUE = 'pushint 11064575'
GREEN = 'pushint 8191920'
def MK(x, y, w, h, size, color, align):
    return CP(T, 'hbxMkText', N(x), N(y), N(w), N(h), N(size), color, f'pushstring "{align}"')
ROWS = GP(T, 'hbxRows')

# a text field in the menu's embedded font
mktext = ops(J(
    f'findpropstrict {TF}', f'constructprop {TF}, 0', 'setlocal 8',
    SP('getlocal 8', 'selectable', 'pushfalse'),
    SP('getlocal 8', 'mouseEnabled', 'pushfalse'),
    SP('getlocal 8', 'wordWrap', 'pushtrue'),
    SP('getlocal 8', 'x', 'getlocal 1'), SP('getlocal 8', 'y', 'getlocal 2'),
    SP('getlocal 8', 'width', 'getlocal 3'), SP('getlocal 8', 'height', 'getlocal 4'),
    GP(GP(GP(T, 'titleLabel'), 'textField'), 'defaultTextFormat'), 'setlocal 9',
    SP('getlocal 9', 'size', 'getlocal 5'), SP('getlocal 9', 'align', 'getlocal 7'),
    SP('getlocal 8', 'defaultTextFormat', 'getlocal 9'),
    SP('getlocal 8', 'textColor', 'getlocal 6'),
    'getlocal 8', 'returnvalue'))

def card_text(x, y, w, h, size, color):
    return CV('getlocal3', 'addChild', MK(x, y, w, h, size, color, 'left'))

def header(x, y, w, h, size, color, align, text, store=None):
    out = [MK(x, y, w, h, size, color, align), 'setlocal 4', SP('getlocal 4', 'text', f'pushstring "{text}"')]
    if store: out.append(SP(T, store, 'getlocal 4'))
    out.append(CV('getlocal1', 'addChild', 'getlocal 4'))
    return J(*out)

gridmake = ops(J(
    GP(T, 'hbxGrid'), 'iftrue LEND',
    f'findpropstrict {SPRITE}', f'constructprop {SPRITE}, 0', 'setlocal1',
    SP(T, 'hbxGrid', 'getlocal1'),
    GP('getlocal1', 'graphics'), 'setlocal 6',
    CV('getlocal 6', 'beginFill', N(0), N(0)),
    CV('getlocal 6', 'drawRect', N(0), N(0), N(1100), N(650)),
    CV('getlocal 6', 'endFill'),
    SP('getlocal1', 'visible', 'pushfalse'),
    'newarray 0', 'setlocal 5',
    SP(T, 'hbxCards', 'getlocal 5'),
    T, 'newobject 0', f'setproperty {q("hbxAsked")}',
    N(0), 'setlocal 2', 'jump LCOND',
    'LBODY:', 'label',
    f'findpropstrict {SPRITE}', f'constructprop {SPRITE}, 0', 'setlocal 3',
    SP('getlocal 3', 'x', J(N(24), 'getlocal 2', N(4), 'modulo', N(266), 'multiply', 'add')),
    SP('getlocal 3', 'y', J(N(110), 'getlocal 2', N(4), 'divide', 'convert_i', N(166), 'multiply', 'add')),
    SP('getlocal 3', 'buttonMode', 'pushtrue'),
    SP('getlocal 3', 'mouseChildren', 'pushfalse'),
    CV('getlocal 3', 'addEventListener', 'pushstring "click"', GP(T, 'hbxCardClick')),
    card_text(120, 8, 124, 66, 16, WHITE),
    card_text(120, 76, 124, 40, 13, BLUE),
    card_text(10, 120, 230, 24, 15, GREEN),
    f'findpropstrict {IL}', f'constructprop {IL}, 0', 'setlocal 7',
    f'findpropstrict {SPRITE}', f'constructprop {SPRITE}, 0', 'setlocal 8',
    GP('getlocal 8', 'graphics'), 'setlocal 6',
    CV('getlocal 6', 'beginFill', N(0), N(0)),
    CV('getlocal 6', 'drawRect', N(0), N(0), N(100), N(100)),
    CV('getlocal 6', 'endFill'),
    SP('getlocal 7', 'imageContainer', 'getlocal 8'),
    CV('getlocal 7', 'addChild', 'getlocal 8'),
    SP('getlocal 7', 'x', N(10)), SP('getlocal 7', 'y', N(10)),
    CV('getlocal 3', 'addChild', 'getlocal 7'),
    CV('getlocal 5', 'push', 'getlocal 3').replace(q('push'), BI('push')),
    CV('getlocal1', 'addChild', 'getlocal 3'),
    'inclocal_i 2',
    'LCOND:', 'getlocal 2', N(12), 'iflt LBODY',
    header(330, 22, 440, 40, 28, WHITE, 'center', 'WORKSHOP DOWNLOADER'),
    header(330, 66, 440, 28, 16, BLUE, 'center', 'Download a workshop map directly into your game!'),
    header(800, 40, 276, 30, 18, WHITE, 'right', '', 'hbxPageLbl'),
    header(24, 612, 1052, 26, 15, BLUE, 'center', 'A  DOWNLOAD      LB / RB  PAGE      Y  SEARCH      B  BACK'),
    'LEND:'))

griddraw = ops(J(
    GP(ROWS, 'length'), 'convert_i', 'setlocal 6',
    N(0), 'setlocal1', 'jump LCOND',
    'LBODY:', 'label',
    GP(T, 'hbxCards'), 'getlocal1', f'getproperty {DYN}', 'setlocal2',
    GP(T, 'hbxPage'), N(12), 'multiply', 'getlocal1', 'add', 'convert_i', 'setlocal 4',
    'getlocal 4', 'getlocal 6', 'iflt LSHOW',
    SP('getlocal2', 'visible', 'pushfalse'), 'jump LNEXT',
    'LSHOW:',
    SP('getlocal2', 'visible', 'pushtrue'),
    ROWS, 'getlocal 4', f'getproperty {DYN}', 'setlocal3',
    CP('getlocal2', 'getChildAt', N(0)), GP('getlocal3', 'Title'), 'convert_s', f'setproperty {q("text")}',
    CP('getlocal2', 'getChildAt', N(1)), GP('getlocal3', 'AuthorName'), 'convert_s', f'setproperty {q("text")}',
    GP(T, 'hbxAsked'), GP('getlocal3', 'FullPath'), f'getproperty {DYN}', 'iffalse LNOTASK',
    'pushstring "REQUESTED"', 'setlocal 12', 'jump LSET',
    'LNOTASK:', 'pushstring "A  DOWNLOAD"', 'setlocal 12',
    'LSET:',
    CP('getlocal2', 'getChildAt', N(2)), 'getlocal 12', f'setproperty {q("text")}',
    GP('getlocal3', 'Preview'), 'iffalse LNOPV',
    GP('getlocal3', 'Preview'), 'convert_s', 'setlocal 13', 'jump LPV',
    'LNOPV:', 'pushstring ""', 'setlocal 13',
    'LPV:',
    CP('getlocal2', 'getChildAt', N(3)), 'getlocal 13', f'setproperty {q("image")}',
    'getlocal1', GP(T, 'hbxSel'), 'ifne LNOSEL',
    N(4), 'setlocal 8', 'pushint 16777215', 'setlocal 9', 'pushint 24462', 'setlocal 10', 'jump LDRAW',
    'LNOSEL:', N(2), 'setlocal 8', 'pushint 8180735', 'setlocal 9', 'pushint 666180', 'setlocal 10',
    'LDRAW:',
    GP('getlocal2', 'graphics'), 'setlocal 5',
    CV('getlocal 5', 'clear'),
    CV('getlocal 5', 'beginFill', 'getlocal 10', N(1)),
    CV('getlocal 5', 'lineStyle', 'getlocal 8', 'getlocal 9'),
    CV('getlocal 5', 'drawRect', N(0), N(0), N(250), N(150)),
    CV('getlocal 5', 'endFill'),
    CV('getlocal 5', 'lineStyle'),
    CV('getlocal 5', 'beginFill', 'pushint 266268', N(1)),
    CV('getlocal 5', 'drawRect', N(10), N(10), N(100), N(100)),
    CV('getlocal 5', 'endFill'),
    'LNEXT:', 'inclocal_i 1',
    'LCOND:', 'getlocal1', N(12), 'iflt LBODY',
    'getlocal 6', N(11), 'add', N(12), 'divide', 'convert_i', 'setlocal 11',
    'getlocal 11', N(1), 'ifge LPG', N(1), 'setlocal 11', 'LPG:',
    SP(GP(T, 'hbxPageLbl'), 'text', J(
        'pushstring "PAGE "', GP(T, 'hbxPage'), N(1), 'add', 'add',
        'pushstring " / "', 'add', 'getlocal 11', 'add',
        'pushstring "   "', 'add', 'getlocal 6', 'add', 'pushstring " MAPS"', 'add'))))

gridrows = ops(J(
    f'getlex {fq("tagame.api","WorkshopItems")}', f'callproperty {q("getAllRows")}, 0', f'coerce {q("Array")}', 'setlocal1',
    'pushstring ""', 'setlocal 2',
    GP(T, 'hbxSearch'), 'iffalse LNOQ',
    GP(GP(T, 'hbxSearch'), 'text'), 'convert_s', f'callproperty {BI("toLowerCase")}, 0', 'setlocal 2',
    'LNOQ:',
    'newarray 0', 'setlocal 3', N(0), 'setlocal 4', 'jump LCOND',
    'LBODY:', 'label',
    'getlocal1', 'getlocal 4', f'getproperty {DYN}', 'setlocal 5',
    GP('getlocal 5', 'Filename'), 'convert_s', 'pushstring "D"', f'callproperty {BI("indexOf")}, 1', N(0), 'iflt LNEXT',
    GP('getlocal 2', 'length'), N(0), 'ifeq LKEEP',
    GP('getlocal 5', 'Title'), 'convert_s', f'callproperty {BI("toLowerCase")}, 0', 'getlocal 2', f'callproperty {BI("indexOf")}, 1', N(0), 'iflt LNEXT',
    'LKEEP:', 'getlocal 3', 'getlocal 5', f'callpropvoid {BI("push")}, 1',
    'LNEXT:', 'inclocal_i 4',
    'LCOND:', 'getlocal 4', GP('getlocal1', 'length'), 'iflt LBODY',
    SP(T, 'hbxRows', 'getlocal 3'),
    CV(T, 'hbxGridSet', N(0), N(0))))

gridset = ops(J(
    GP(ROWS, 'length'), 'convert_i', 'setlocal 3',
    'getlocal 3', N(11), 'add', N(12), 'divide', 'convert_i', 'setlocal 4',
    'getlocal 4', N(1), 'ifge L0', N(1), 'setlocal 4', 'L0:',
    'getlocal 1', N(0), 'ifge L1', N(0), 'setlocal 1', 'L1:',
    'getlocal 1', 'getlocal 4', N(1), 'subtract', 'ifle L2', 'getlocal 4', N(1), 'subtract', 'setlocal 1', 'L2:',
    'getlocal 3', N(1), 'subtract', 'getlocal 1', N(12), 'multiply', 'subtract', 'setlocal 5',
    'getlocal 5', N(11), 'ifle L3', N(11), 'setlocal 5', 'L3:',
    'getlocal 5', N(0), 'ifge L4', N(0), 'setlocal 5', 'L4:',
    'getlocal 2', 'getlocal 5', 'ifle L5', 'getlocal 5', 'setlocal 2', 'L5:',
    'getlocal 2', N(0), 'ifge L6', N(0), 'setlocal 2', 'L6:',
    SP(T, 'hbxPage', J('getlocal 1', 'convert_i')),
    SP(T, 'hbxSel', J('getlocal 2', 'convert_i')),
    CV(T, 'hbxGridDraw')))

gridgo = ops(J(
    GP(T, 'hbxPage'), N(12), 'multiply', GP(T, 'hbxSel'), 'add', 'convert_i', 'setlocal1',
    'getlocal1', GP(ROWS, 'length'), 'ifge LEND',
    ROWS, 'getlocal1', f'getproperty {DYN}', 'setlocal2',
    GP(T, 'hbxAsked'), GP('getlocal2', 'FullPath'), f'getproperty {DYN}', 'iftrue LEND',
    CV(T, 'hbxDownload', 'getlocal2'),
    'LEND:'))

cardclick = ops(J(
    GP(T, 'hbxCards'), GP('getlocal1', 'currentTarget'), f'callproperty {BI("indexOf")}, 1', 'convert_i', 'setlocal2',
    'getlocal2', N(0), 'iflt LEND',
    CV(T, 'hbxGridSet', GP(T, 'hbxPage'), 'getlocal2'),
    CV(T, 'hbxGridGo'),
    'LEND:'))

def SETPAGE(dp, ds):
    return CV(T, 'hbxGridSet', J(GP(T, 'hbxPage'), *dp), J(GP(T, 'hbxSel'), *ds))

gridkey = ops(J(
    'getlocal2', IV('KEY_UP'), 'equals', 'setlocal 4',
    CP(T, 'hbxTyping'), 'setlocal 5',
    'getlocal 5', 'iffalse LNOTYPE',
    'getlocal2', IV('KEY_DOWN'), 'ifne LEND',
    'getlocal1', NAV('ESCAPE'), 'ifeq LBLUR',
    'getlocal1', NAV('DOWN'), 'ifeq LBLUR',
    'getlocal1', NAV('ENTER'), 'ifne LEND',
    'getlocal3', N(32), 'ifeq LEND',
    'LBLUR:', SP(GP(T, 'stage'), 'focus', 'pushnull'), 'jump LEND',
    'LNOTYPE:',
    'getlocal1', NAV('ESCAPE'), 'ifne LNOTB',
    'getlocal 4', 'iffalse LEND', CV(T, 'hbxToggleDl'), 'jump LEND',
    'LNOTB:',
    'getlocal1', NAV('ENTER'), 'ifne LNOTA',
    'getlocal 4', 'iffalse LEND', CV(T, 'hbxGridGo'), 'jump LEND',
    'LNOTA:',
    'getlocal1', NAV('GAMEPAD_Y'), 'ifne LNOTY',
    'getlocal 4', 'iffalse LEND', CV(T, 'hbxKeyboard'), 'jump LEND',
    'LNOTY:',
    'getlocal 4', 'iftrue LEND',
    'getlocal1', NAV('GAMEPAD_L1'), 'ifeq LPREV',
    'getlocal1', NAV('PAGE_UP'), 'ifne LNOTP',
    'LPREV:', SETPAGE([N(1), 'subtract'], []), 'jump LEND',
    'LNOTP:',
    'getlocal1', NAV('GAMEPAD_R1'), 'ifeq LNEXTP',
    'getlocal1', NAV('PAGE_DOWN'), 'ifne LNOTN',
    'LNEXTP:', SETPAGE([N(1), 'add'], []), 'jump LEND',
    'LNOTN:',
    'getlocal1', NAV('LEFT'), 'ifne LNOTL',
    SETPAGE([], [N(1), 'subtract']), 'jump LEND',
    'LNOTL:',
    'getlocal1', NAV('RIGHT'), 'ifne LNOTR',
    SETPAGE([], [N(1), 'add']), 'jump LEND',
    'LNOTR:',
    'getlocal1', NAV('UP'), 'ifne LNOTU',
    GP(T, 'hbxSel'), N(4), 'iflt LEND',
    SETPAGE([], [N(4), 'subtract']), 'jump LEND',
    'LNOTU:',
    'getlocal1', NAV('DOWN'), 'ifne LEND',
    GP(T, 'hbxSel'), N(7), 'ifgt LEND',
    SETPAGE([], [N(4), 'add']),
    'LEND:'))

typing = ops(J(
    GP(GP(T, 'stage'), 'focus'), 'setlocal1',
    'getlocal1', 'iffalse LNO',
    GP(T, 'hbxSearch'), 'iffalse LNO',
    'getlocal1', GP(T, 'hbxSearch'), 'ifeq LYES',
    GP('getlocal1', 'parent'), GP(T, 'hbxSearch'), 'ifeq LYES',
    'LNO:', 'pushfalse', 'returnvalue',
    'LYES:', 'pushtrue', 'returnvalue'))

PIECES = ['workshopItemsList', 'scrollBar', 'closeButton', 'titleLabel', 'loadingLabel']
def show(flag):
    out = [SP(GP(T, p), 'visible', flag) for p in PIECES]
    out += [GP(T, 'hbxLabel'), 'iffalse LNL', SP(GP(T, 'hbxLabel'), 'visible', flag), 'LNL:']
    return J(*out)

gridopen = ops(J(
    CP(T, 'hbxCountDl'), N(0), 'ifngt LEND',
    CV(T, 'hbxGridMake'),
    SP(T, 'hbxDl', 'pushtrue'),
    show('pushfalse'),
    f'getlex {q("Math")}', N(1), f'getlex {q("Math")}',
    f'getlex {UIS}', f'getproperty {q("ContentStageWidth")}', N(60), 'subtract', N(1100), 'divide',
    f'getlex {UIS}', f'getproperty {q("ContentStageHeight")}', N(60), 'subtract', N(650), 'divide',
    f'callproperty {q("min")}, 2', f'callproperty {q("min")}, 2', 'setlocal1',
    GP(T, 'hbxGrid'), 'setlocal2',
    SP('getlocal2', 'scaleX', 'getlocal1'), SP('getlocal2', 'scaleY', 'getlocal1'),
    SP('getlocal2', 'x', J(N(-550), 'getlocal1', 'multiply')),
    SP('getlocal2', 'y', J(f'getlex {UIS}', f'getproperty {q("ContentStageHeight")}', 'pushdouble 0.5', 'multiply',
                          GP(T, 'y'), 'subtract', N(325), 'getlocal1', 'multiply', 'subtract')),
    SP(GP(T, 'background'), 'width', J(N(1100), 'getlocal1', 'multiply')),
    SP(GP(T, 'background'), 'height', J(N(650), 'getlocal1', 'multiply')),
    SP(GP(T, 'background'), 'x', J(N(-550), 'getlocal1', 'multiply')),
    SP(GP(T, 'background'), 'y', GP('getlocal2', 'y')),
    CV(T, 'addChild', 'getlocal2'),
    SP('getlocal2', 'visible', 'pushtrue'),
    GP(T, 'hbxSearch'), 'iffalse LNS',
    SP(GP(T, 'hbxSearch'), 'x', J(GP('getlocal2', 'x'), N(24), 'getlocal1', 'multiply', 'add')),
    SP(GP(T, 'hbxSearch'), 'y', J(GP('getlocal2', 'y'), N(28), 'getlocal1', 'multiply', 'add')),
    CV(T, 'addChild', GP(T, 'hbxSearch')),
    'LNS:',
    CV(T, 'hbxGridRows'),
    'LEND:'))

gridclose = ops(J(
    SP(T, 'hbxDl', 'pushfalse'),
    SP(GP(T, 'hbxGrid'), 'visible', 'pushfalse'),
    show('pushtrue'),
    SP(GP(T, 'hbxSearch'), 'text', 'pushstring ""'),
    T, f'callpropvoid QName(ProtectedNamespace("menus:WorkshopMenu"), "handleWorkshopItemsChanged"), 0',
    CV(T, 'hbxMake')))

keyboard = ops(J(
    SP(f'getlex {SHELL}', 'OnShowKeyboardComplete', GP(T, 'hbxKbDone')),
    f'getlex {SHELL}', 'pushstring "SEARCH"', 'pushstring ""', 'pushfalse',
    GP(GP(T, 'hbxSearch'), 'text'), 'convert_s', N(40),
    f'callproperty {q("ShowKeyboard")}, 5', 'pop'))

kbdone = ops(J(
    GP(T, 'hbxDl'), 'iffalse LEND',
    SP(GP(T, 'hbxSearch'), 'text', 'getlocal1'),
    CV(T, 'hbxGridRows'),
    'LEND:'))

vis = ops(J(
    'newarray 0', 'setlocal2', N(0), 'setlocal3', 'jump LCOND',
    'LBODY:', 'label',
    'getlocal1', 'getlocal3', f'getproperty {DYN}', 'setlocal 4',
    GP('getlocal 4', 'Filename'), 'convert_s', 'pushstring "D"', f'callproperty {BI("indexOf")}, 1', N(0), 'ifge LNEXT',
    'getlocal2', 'getlocal 4', f'callpropvoid {BI("push")}, 1',
    'LNEXT:', 'inclocal_i 3',
    'LCOND:', 'getlocal3', GP('getlocal1', 'length'), 'iflt LBODY',
    'getlocal2', 'returnvalue'))

countdl = ops(J(
    f'getlex {fq("tagame.api","WorkshopItems")}', f'callproperty {q("getAllRows")}, 0', f'coerce {q("Array")}', 'setlocal1',
    N(0), 'setlocal2', N(0), 'setlocal3', 'jump LCOND',
    'LBODY:', 'label',
    'getlocal1', 'getlocal2', f'getproperty {DYN}', GP('', 'Filename').lstrip('\n'), 'convert_s', 'pushstring "D"',
    f'callproperty {BI("indexOf")}, 1', N(0), 'iflt LNEXT',
    'inclocal_i 3',
    'LNEXT:', 'inclocal_i 2',
    'LCOND:', 'getlocal2', GP('getlocal1', 'length'), 'iflt LBODY',
    'getlocal3', 'returnvalue'))

toggledl = ops(J(
    GP(T, 'hbxDl'), 'iftrue LOFF',
    CV(T, 'hbxGridOpen'), 'jump LEND',
    'LOFF:', CV(T, 'hbxGridClose'),
    'LEND:'))

dlkey = ops(J(CV(T, 'hbxToggleDl')))

download = ops(J(
    'getlocal1', 'iffalse LEND',
    log('HBXDL', J('getlocal1', f'getproperty {q("FullPath")}')),
    ''.join(log('HBXPAD', 'pushstring "' + ' ' * 900 + '"') for _ in range(6)),
    GP(T, 'hbxAsked'), GP('getlocal1', 'FullPath'), 'pushtrue', f'setproperty {DYN}',
    CV(T, 'hbxGridDraw'),
    'LEND:'))

mmethods=(method('hbxState',['null'],state,4,3,ret='*')
 +method('hbxToggleRow',['null'],toggle,6,4)
 +method('hbxHost',['null'],host,4,2)
 +method('hbxPath',['null'],path,4,2,ret='*')
 +method('hbxDraw',[SPRITE,q('Boolean')],draw,6,4)
 +method('hbxInit',[],init,6,4)
 +method('hbxUpdate',['null'],update,6,9)
 +method('hbxBoxClick',['null'],click,5,3)
 +method('hbxLayout',[],layout,5,1)
 +method('hbxInput',['null'],INPUT,6,5)
 +method('hbxRight',[fq('scaleform.clik.events','ListEvent')],right,3,2)
 +method('hbxHostKey',['null'],hostkey,4,2)
 +method('hbxMake',[],make,7,4)
 +method('hbxFilter',['null'],filt,6,7)
 +method('hbxSearchFocus',['null'],focus,4,2)
 +method('hbxSearchBlur',['null'],blur,4,2)
 +method('hbxIcon',[],icon,6,5)
 +method('hbxLegendOn',[],legon,8,2)
 +method('hbxLegendOff',[],legoff,4,2)
 +method('hbxInputChanged',[],inputchg,4,1)
 +method('hbxVisible',['null'],vis,5,5,ret='*')
 +method('hbxCountDl',[],countdl,4,4,ret='int')
 +method('hbxToggleDl',[],toggledl,4,1)
 +method('hbxDlKey',['null'],dlkey,2,2)
 +method('hbxDownload',['null'],download,8,2)
 +method('hbxMkText',['null']*7,mktext,6,10,ret='*')
 +method('hbxGridMake',[],gridmake,10,9)
 +method('hbxGridDraw',[],griddraw,10,14)
 +method('hbxGridRows',[],gridrows,6,6)
 +method('hbxGridSet',['null','null'],gridset,5,6)
 +method('hbxGridGo',[],gridgo,5,3)
 +method('hbxCardClick',['null'],cardclick,5,3)
 +method('hbxGridKey',['null','null','null'],gridkey,8,6)
 +method('hbxTyping',[],typing,4,2,ret='*')
 +method('hbxGridOpen',[],gridopen,10,3)
 +method('hbxGridClose',[],gridclose,5,1)
 +method('hbxKeyboard',[],keyboard,8,1)
 +method('hbxKbDone',['null'],kbdone,5,2))
mslots=(f'  trait slot {q("hbxMode")} type {q("int")} end\n'
 f'  trait slot {q("hbxOnBox")} type {q("Boolean")} end\n'
 f'  trait slot {q("hbxSearch")} type null end\n'
 f'  trait slot {q("hbxLabel")} type {fq("flash.text","TextField")} end\n'
 f'  trait slot {q("hbxBoxes")} type {q("Array")} end\n'
 f'  trait slot {q("hbxShown")} type {q("Array")} end\n'
 f'  trait slot {q("hbxTicks")} type {q("Object")} end\n'
 f'  trait slot {q("hbxDl")} type {q("Boolean")} end\n'
 f'  trait slot {q("hbxLegDl")} type {q("Boolean")} end\n'
 f'  trait slot {q("hbxGrid")} type {SPRITE} end\n'
 f'  trait slot {q("hbxCards")} type {q("Array")} end\n'
 f'  trait slot {q("hbxRows")} type {q("Array")} end\n'
 f'  trait slot {q("hbxPageLbl")} type {fq("flash.text","TextField")} end\n'
 f'  trait slot {q("hbxPage")} type {q("int")} end\n'
 f'  trait slot {q("hbxSel")} type {q("int")} end\n'
 f'  trait slot {q("hbxAsked")} type {q("Object")} end\n')
LS='QName(ProtectedNamespace("menus:WorkshopMenu"), "handleIndexChange")\n      callpropvoid        QName(PackageNamespace(""), "addEventListener"), 2\n'
LISTEN=f'''
      getlocal0
      getproperty         {q("workshopItemsList")}
      getlex              {fq("scaleform.clik.events","ListEvent")}
      getproperty         {q("ITEM_RIGHT_CLICK")}
      getlocal0
      getproperty         {q("hbxRight")}
      callpropvoid        {q("addEventListener")}, 2
'''
def inp(kind):
    return f'''
      getlex              {fq("scaleform.clik.managers","InputDelegate")}
      callproperty        {q("getInstance")}, 0
      getlex              {fq("scaleform.clik.events","InputEvent")}
      getproperty         {q("INPUT")}
      getlocal0
      getproperty         {q("hbxInput")}
      pushfalse
      pushbyte            100
      pushfalse
      callpropvoid        {q(kind)}, 5
'''
def inp_off():
    return f'''
      getlex              {fq("scaleform.clik.managers","InputDelegate")}
      callproperty        {q("getInstance")}, 0
      getlex              {fq("scaleform.clik.events","InputEvent")}
      getproperty         {q("INPUT")}
      getlocal0
      getproperty         {q("hbxInput")}
      callpropvoid        {q("removeEventListener")}, 3
'''.replace('callpropvoid        '+q("removeEventListener")+', 3','pushfalse\n      callpropvoid        '+q("removeEventListener")+', 3')
RESIZE='      getlocal0\n      callpropvoid        QName(ProtectedNamespace("menus:WorkshopMenu"), "handleStageResized"), 0\n\n      returnvoid\n'
TO='callpropvoid        QName(PackageNamespace(""), "to"), 3\n\n      returnvoid\n'
menu='menus/WorkshopMenu.class.asasm'
WIC='      coerce              QName(PackageNamespace(""), "Array")\n      setlocal1\n\n      getlocal0\n      getproperty         QName(PackageNamespace(""), "workshopItemsList")\n      findpropstrict      QName(PackageNamespace("scaleform.clik.data"), "DataProvider")\n'
WIC_NEW=WIC.replace('      getlocal0\n', f'      getlocal0\n      getlocal1\n      callproperty        {q("hbxVisible")}, 1\n      coerce              {q("Array")}\n      setlocal1\n\n      getlocal0\n', 1)
GETLEX_WS='      getlex              QName(PackageNamespace("tagame.api"), "Workshop")\n'
CLICK_OLD=GETLEX_WS+'      getlocal2\n      getproperty         QName(PackageNamespace(""), "FullPath")\n      callpropvoid        QName(PackageNamespace(""), "LoadWorkshopMap"), 1\n'
CLICK_NEW=f'      getlocal0\n      getproperty         {q("hbxDl")}\n      iffalse             LHBXNODL\n      jump                L14\n\nLHBXNODL:\n'+GETLEX_WS+'      getlocal2\n      getproperty         QName(PackageNamespace(""), "Filename")\n      pushstring          "h"\n      callproperty        QName(Namespace("http://adobe.com/AS3/2006/builtin"), "indexOf"), 1\n      pushbyte            0\n      ifnge               LHBXPLAIN\n\n      getlocal0\n      getlocal2\n      callproperty        QName(PackageNamespace(""), "hbxPath"), 1\n      jump                LHBXGO\n\nLHBXPLAIN:\n      getlocal2\n      getproperty         QName(PackageNamespace(""), "FullPath")\n\nLHBXGO:\n      callpropvoid        QName(PackageNamespace(""), "LoadWorkshopMap"), 1\n'
HDR='     maxstack 2\n     localcount 3\n     initscopedepth 13\n     maxscopedepth 14\n     code\n      getlocal0\n      pushscope\n\n      getlocal1\n      getproperty         QName(PackageNamespace(""), "itemData")'
HEAD='     maxstack 3\n     localcount 1\n     initscopedepth 13\n     maxscopedepth 14\n     code\n      getlocal0\n      pushscope\n\n      getlocal0\n      callsupervoid       QName(ProtectedNamespace("projectx.core:UIMenu"), "%s"), 0'
STACK_ON=HEAD % "addEventListeners"
STACK_OFF=HEAD % "removeEventListeners"
build('v23',{
 menu:[
  (LS,LS+LISTEN),
  (CLICK_OLD,CLICK_NEW),
  (WIC,WIC_NEW),
  (HDR,HDR.replace('maxstack 2','maxstack 4')),
  (STACK_ON, STACK_ON.replace('maxstack 3','maxstack 7')),
  (STACK_OFF, STACK_OFF.replace('maxstack 3','maxstack 7')),
  (' end ; instance\n',mmethods+' end ; instance\n'),
  ('  trait method QName(ProtectedNamespace("menus:WorkshopMenu"), "configUI")',mslots+'  trait method QName(ProtectedNamespace("menus:WorkshopMenu"), "configUI")'),
  (RESIZE,f'      getlocal0\n      callpropvoid        {q("hbxInit")}, 0\n      getlocal0\n      callpropvoid        {q("hbxLayout")}, 0\n\n'+RESIZE),
  (TO,f'callpropvoid        QName(PackageNamespace(""), "to"), 3\n\n      getlocal0\n      callpropvoid        {q("hbxInputChanged")}, 0\n\n      returnvoid\n'),
  ('"addEventListener"), 2\n\nL13:\n      returnvoid\n','"addEventListener"), 2\n\nL13:\n'+inp('addEventListener')+'      returnvoid\n'),
  ('"removeEventListener"), 2\n\nL13:\n      returnvoid\n','"removeEventListener"), 2\n\nL13:\n'+inp_off()+'      getlocal0\n      callpropvoid        '+q('hbxLegendOff')+', 0\n      returnvoid\n'),
 ],
})
