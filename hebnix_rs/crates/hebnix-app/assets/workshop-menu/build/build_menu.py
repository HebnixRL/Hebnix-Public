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
getlocal 4
getlocal3
convert_b
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
pushstring "LAN READY"
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
getlex QName(PackageNamespace("tagame.api"), "WorkshopItems")
callproperty QName(PackageNamespace(""), "getAllRows"), 0
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
 +method('hbxLegendOn',[],legon,8,1)
 +method('hbxLegendOff',[],legoff,4,1)
 +method('hbxInputChanged',[],inputchg,4,1))
mslots=(f'  trait slot {q("hbxMode")} type {q("int")} end\n'
 f'  trait slot {q("hbxOnBox")} type {q("Boolean")} end\n'
 f'  trait slot {q("hbxSearch")} type null end\n'
 f'  trait slot {q("hbxLabel")} type {fq("flash.text","TextField")} end\n'
 f'  trait slot {q("hbxBoxes")} type {q("Array")} end\n'
 f'  trait slot {q("hbxShown")} type {q("Array")} end\n'
 f'  trait slot {q("hbxTicks")} type {q("Object")} end\n')
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
CLICK_OLD='      getlocal2\n      getproperty         QName(PackageNamespace(""), "FullPath")\n      callpropvoid        QName(PackageNamespace(""), "LoadWorkshopMap"), 1\n'
CLICK_NEW='      getlocal2\n      getproperty         QName(PackageNamespace(""), "Filename")\n      pushstring          "h"\n      callproperty        QName(Namespace("http://adobe.com/AS3/2006/builtin"), "indexOf"), 1\n      pushbyte            0\n      ifnge               LHBXPLAIN\n\n      getlocal0\n      getlocal2\n      callproperty        QName(PackageNamespace(""), "hbxPath"), 1\n      jump                LHBXGO\n\nLHBXPLAIN:\n      getlocal2\n      getproperty         QName(PackageNamespace(""), "FullPath")\n\nLHBXGO:\n      callpropvoid        QName(PackageNamespace(""), "LoadWorkshopMap"), 1\n'
HDR='     maxstack 2\n     localcount 3\n     initscopedepth 13\n     maxscopedepth 14\n     code\n      getlocal0\n      pushscope\n\n      getlocal1\n      getproperty         QName(PackageNamespace(""), "itemData")'
HEAD='     maxstack 3\n     localcount 1\n     initscopedepth 13\n     maxscopedepth 14\n     code\n      getlocal0\n      pushscope\n\n      getlocal0\n      callsupervoid       QName(ProtectedNamespace("projectx.core:UIMenu"), "%s"), 0'
STACK_ON=HEAD % "addEventListeners"
STACK_OFF=HEAD % "removeEventListeners"
build('v18',{
 menu:[
  (LS,LS+LISTEN),
  (CLICK_OLD,CLICK_NEW),
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
