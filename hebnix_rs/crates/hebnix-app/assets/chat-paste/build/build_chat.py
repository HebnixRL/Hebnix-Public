# Chat paste HUD movie: Ctrl+V in the match chat box inserts the clipboard (Shell.PasteFromClipboard) at the caret.
# The chat classes also live in the shared code library, which loads first and wins, so the hook goes in
# HudScreen (only in this movie): a capture listener on its chat log view.
import shutil, os, subprocess, sys
R = os.path.expanduser('~/Documents/hebnix-rl-re/tools/rabc')
M = 'HudScreen'
def q(n): return f'QName(PackageNamespace(""), "{n}")'
def fq(p, n): return f'QName(PackageNamespace("{p}"), "{n}")'
def bq(n): return f'QName(Namespace("http://adobe.com/AS3/2006/builtin"), "{n}")'

def ops(text):
    out = ''
    for l in text.strip().splitlines():
        l = l.strip()
        if l:
            out += (l + '\n') if l.endswith(':') else '      ' + l + '\n'
    return out

# The game never sets KeyboardEvent.ctrlKey, but Ctrl (17) arrives as its own key down, so hbxCtrl
# holds it from Ctrl's key down to its key up (or any key other than V, if the key up never comes).
# locals: 1 event, 2 clipboard, 3 caret, 4 textField, 5 old text
PASTE = rf'''
getlocal1
getproperty {q("keyCode")}
pushbyte 17
ifne LHBXNOTCTRL
getlocal0
pushtrue
setproperty {q("hbxCtrl")}
returnvoid
LHBXNOTCTRL:
getlocal1
getproperty {q("keyCode")}
pushbyte 86
ifeq LHBXV
getlocal0
pushfalse
setproperty {q("hbxCtrl")}
returnvoid
LHBXV:
getlocal1
getproperty {q("ctrlKey")}
iftrue LHBXCTRL
getlocal0
getproperty {q("hbxCtrl")}
iffalse LHBXEND
LHBXCTRL:
getlex {fq("tagame.api", "Shell")}
callproperty {q("PasteFromClipboard")}, 0
coerce_s
setlocal2
getlocal2
pushnull
ifeq LHBXEND
getlocal1
getproperty {q("target")}
istype {fq("flash.text", "TextField")}
iffalse LHBXEND
getlocal2
pushstring "\r"
callproperty {bq("split")}, 1
pushstring ""
callproperty {bq("join")}, 1
pushstring "\n"
callproperty {bq("split")}, 1
pushstring " "
callproperty {bq("join")}, 1
coerce_s
setlocal2
getlocal1
getproperty {q("target")}
setlocal 4
getlocal 4
getproperty {q("selectionBeginIndex")}
convert_i
setlocal3
getlocal 4
getproperty {q("text")}
coerce_s
setlocal 5
getlocal 4
getproperty {q("maxChars")}
pushbyte 0
ifngt LHBXPUT
getlocal2
pushbyte 0
getlocal 4
getproperty {q("maxChars")}
getlocal 5
getproperty {q("length")}
subtract
getlocal 4
getproperty {q("selectionEndIndex")}
add
getlocal3
subtract
callproperty {bq("substr")}, 2
coerce_s
setlocal2
LHBXPUT:
getlocal 4
getlocal 5
pushbyte 0
getlocal3
callproperty {bq("substr")}, 2
getlocal2
add
getlocal 5
getlocal 4
getproperty {q("selectionEndIndex")}
callproperty {bq("substr")}, 1
add
setproperty {q("text")}
getlocal3
getlocal2
getproperty {q("length")}
add
convert_i
setlocal3
getlocal 4
getlocal3
getlocal3
callpropvoid {q("setSelection")}, 2
getlocal 4
findpropstrict {fq("flash.events", "Event")}
pushstring "change"
constructprop {fq("flash.events", "Event")}, 1
callpropvoid {q("dispatchEvent")}, 1
getlocal1
callpropvoid {q("stopImmediatePropagation")}, 0
LHBXEND:
'''

METHOD = f'''  trait method {q("hbxPaste")}
   method
    refid "{M}/instance/hbxPaste"
    param {fq("flash.events", "KeyboardEvent")}
    returns {q("void")}
    body
     maxstack 8
     localcount 6
     initscopedepth 13
     maxscopedepth 14
     code
      getlocal0
      pushscope

{ops(PASTE)}      returnvoid
     end ; code
    end ; body
   end ; method
  end ; trait
  trait method {q("hbxKeyUp")}
   method
    refid "{M}/instance/hbxKeyUp"
    param {fq("flash.events", "KeyboardEvent")}
    returns {q("void")}
    body
     maxstack 2
     localcount 2
     initscopedepth 13
     maxscopedepth 14
     code
      getlocal0
      pushscope
      getlocal1
      getproperty {q("keyCode")}
      pushbyte 17
      ifne LHBXUPEND
      getlocal0
      pushfalse
      setproperty {q("hbxCtrl")}
LHBXUPEND:
      returnvoid
     end ; code
    end ; body
   end ; method
  end ; trait
  trait slot {q("hbxCtrl")} type {q("Boolean")} value False() end
'''

def build(name):
    shutil.rmtree(name, ignore_errors=True)
    shutil.copytree('rt', f'{name}/rt')
    f = f'{name}/rt/HudScreen.class.asasm'
    s = open(f, encoding='utf-8').read()
    # in the constructor, right after m_chatLogView = chatLogViewContainer["chatLogView"]
    view = f'QName(ProtectedNamespace("{M}"), "m_chatLogView")'
    init = f'     initproperty        {view}\n'
    assert s.count(init) == 1
    at = s.index(init) + len(init)
    s = s[:at] + f'''
     getlocal0
     getproperty         {view}
     getlex              {fq("flash.events", "KeyboardEvent")}
     getproperty         {q("KEY_DOWN")}
     getlocal0
     getproperty         {q("hbxPaste")}
     pushtrue
     callpropvoid        {q("addEventListener")}, 3
     getlocal0
     getproperty         {view}
     getlex              {fq("flash.events", "KeyboardEvent")}
     getproperty         {q("KEY_UP")}
     getlocal0
     getproperty         {q("hbxKeyUp")}
     pushtrue
     callpropvoid        {q("addEventListener")}, 3
''' + s[at:]
    assert s.count(' end ; instance\n') == 1
    s = s.replace(' end ; instance\n', METHOD + ' end ; instance\n')
    open(f, 'w', encoding='utf-8').write(s)
    shutil.copy('in.swf', f'{name}/o.swf')
    subprocess.run([R + '/rabcasm.exe', f'{name}/rt/in-0.main.asasm'], check=True)
    subprocess.run([R + '/abcreplace.exe', f'{name}/o.swf', '0', f'{name}/rt/in-0.main.abc'], check=True)

build(sys.argv[1] if len(sys.argv) > 1 else 'out')
