import sys
path = sys.argv[1]
s = open(path, encoding='utf-8').read()


def rep(old, new):
    global s
    assert s.count(old) == 1, (s.count(old), old[:80])
    s = s.replace(old, new)


# 1 update: catalogue rows have no tickbox
rep("""LNOROW:
getlocal 4
pushfalse
setproperty {q("visible")}""", """LNOROW:
getlocal0
getproperty {q("hbxDl")}
iffalse LNODL
pushnull
setlocal3
LNODL:
getlocal 4
pushfalse
setproperty {q("visible")}""")

# 2 input: L3 opens the downloader grid, which then takes every key
rep("""getlocal3
getlex {fq("scaleform.clik.constants","NavigationCode")}
getproperty {q("GAMEPAD_Y")}""", """getlocal0
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
getproperty {q("GAMEPAD_Y")}""")

# 3 search keeps to the shown kind of row
rep("""filt=ops(f'''
getlex QName(PackageNamespace("tagame.api"), "WorkshopItems")
callproperty QName(PackageNamespace(""), "getAllRows"), 0
coerce""", """filt=ops(f'''
getlocal0
getlex QName(PackageNamespace("tagame.api"), "WorkshopItems")
callproperty QName(PackageNamespace(""), "getAllRows"), 0
callproperty {q("hbxVisible")}, 1
coerce""")
rep("""filt=ops(f'''
getlocal0
getlex""", """filt=ops(f'''
getlocal0
getproperty {q("hbxDl")}
iffalse LNORM
getlocal0
callpropvoid {q("hbxGridRows")}, 0
jump LEND
LNORM:
getlocal0
getlex""")

# 4 legend: download entry
rep("""iffalse LEND
getlex {fq("projectx.api","ShellAPI")}
getproperty {q("inputType")}
getlex {fq("projectx.api","InputTypes")}
getproperty {q("PC")}
equals
iffalse LPAD""", """iffalse LEND
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
iffalse LPAD""")
rep("""legoff=ops(f'''
getlocal0
getproperty {LEG}
iffalse LEND
""", """legoff=ops(f'''
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
""")
rep("method('hbxLegendOn',[],legon,8,1)", "method('hbxLegendOn',[],legon,8,2)")
rep("method('hbxLegendOff',[],legoff,4,1)", "method('hbxLegendOff',[],legoff,4,2)")


# 5 grid methods
GRID = open("grid_src.py", encoding="utf-8").read().rsplit('mmethods = ("' + '""', 1)[0]
rep("mmethods=(", GRID + "mmethods=(")
rep(" +method('hbxInputChanged',[],inputchg,4,1))", """ +method('hbxInputChanged',[],inputchg,4,1)
 +method('hbxVisible',['null'],vis,5,5,ret='*')
 +method('hbxCountDl',[],countdl,4,4,ret='int')
 +method('hbxToggleDl',[],toggledl,4,1)
 +method('hbxDlKey',['null'],dlkey,2,2)
 +method('hbxDownload',['null'],download,8,2)
 +method('hbxMkText',['null']*8,mktext,6,11,ret='*')
 +method('hbxGridMake',[],gridmake,11,10)
 +method('hbxGridDraw',[],griddraw,10,16)
 +method('hbxGridRows',[],gridrows,6,6)
 +method('hbxGridSet',['null','null'],gridset,5,6)
 +method('hbxGridGo',[],gridgo,5,3)
 +method('hbxCardClick',['null'],cardclick,5,3)
 +method('hbxGridKey',['null','null','null'],gridkey,8,6)
 +method('hbxTyping',[],typing,4,2,ret='*')
 +method('hbxGridOpen',[],gridopen,10,3)
 +method('hbxGridClose',[],gridclose,6,1)
 +method('hbxKeyboard',[],keyboard,8,1)
 +method('hbxKbDone',['null'],kbdone,5,2))""")
rep(r""" f'  trait slot {q("hbxTicks")} type {q("Object")} end\n')""", r""" f'  trait slot {q("hbxTicks")} type {q("Object")} end\n'
 f'  trait slot {q("hbxDl")} type {q("Boolean")} end\n'
 f'  trait slot {q("hbxLegDl")} type {q("Boolean")} end\n'
 f'  trait slot {q("hbxGrid")} type {SPRITE} end\n'
 f'  trait slot {q("hbxCards")} type {q("Array")} end\n'
 f'  trait slot {q("hbxRows")} type {q("Array")} end\n'
 f'  trait slot {q("hbxPageLbl")} type {fq("flash.text","TextField")} end\n'
 f'  trait slot {q("hbxFoot")} type {fq("flash.text","TextField")} end\n'
 f'  trait slot {q("hbxBgSave")} type {q("Array")} end\n'
 f'  trait slot {q("hbxPage")} type {q("int")} end\n'
 f'  trait slot {q("hbxSel")} type {q("int")} end\n'
 f'  trait slot {q("hbxAsked")} type {q("Object")} end\n')""")

# 7 click: the grid is open, so the hidden list takes no picks
rep("CLICK_OLD='      getlocal2", r"""GETLEX_WS='      getlex              QName(PackageNamespace("tagame.api"), "Workshop")\n'
CLICK_OLD=GETLEX_WS+'      getlocal2""")
rep("CLICK_NEW='      getlocal2", r"""CLICK_NEW=f'      getlocal0\n      getproperty         {q("hbxDl")}\n      iffalse             LHBXNODL\n      jump                L14\n\nLHBXNODL:\n'+GETLEX_WS+'      getlocal2""")

# 8 the vanilla list never shows catalogue rows
rep("  (CLICK_OLD,CLICK_NEW),", "  (CLICK_OLD,CLICK_NEW),\n  (WIC,WIC_NEW),")
rep("menu='menus/WorkshopMenu.class.asasm'", r"""menu='menus/WorkshopMenu.class.asasm'
WIC='      coerce              QName(PackageNamespace(""), "Array")\n      setlocal1\n\n      getlocal0\n      getproperty         QName(PackageNamespace(""), "workshopItemsList")\n      findpropstrict      QName(PackageNamespace("scaleform.clik.data"), "DataProvider")\n'
WIC_NEW=WIC.replace('      getlocal0\n', f'      getlocal0\n      getlocal1\n      callproperty        {q("hbxVisible")}, 1\n      coerce              {q("Array")}\n      setlocal1\n\n      getlocal0\n', 1)""")
rep("build('v20',{", "build('v25',{")
open(path, 'w', encoding='utf-8').write(s)
