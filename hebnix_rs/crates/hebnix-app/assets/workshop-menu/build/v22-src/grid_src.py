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
# the game's fonts: headings use the menu title's, card text the list row's own fields
HEAD = GP(GP(T, 'titleLabel'), 'textField')
PROTO = 'getlocal 9'
ROW_TITLE = GP(PROTO, 'titleTextField')
ROW_DESC = GP(PROTO, 'descTextField')
ROW_AUTHOR = GP(PROTO, 'authorNameTextField')
def MK(x, y, w, h, size, color, align, src):
    return CP(T, 'hbxMkText', N(x), N(y), N(w), N(h), N(size), color, f'pushstring "{align}"', src)
# cards: 4 columns by 2 rows, each a preview over the row's title, description and author
PER = 8
CARD_W, CARD_H = 250, 236
PV_W, PV_H = 234, 132
ROWS = GP(T, 'hbxRows')

# a text field in the font of the text field passed last
mktext = ops(J(
    f'findpropstrict {TF}', f'constructprop {TF}, 0', 'setlocal 9',
    SP('getlocal 9', 'selectable', 'pushfalse'),
    SP('getlocal 9', 'mouseEnabled', 'pushfalse'),
    SP('getlocal 9', 'wordWrap', 'pushtrue'),
    SP('getlocal 9', 'x', 'getlocal 1'), SP('getlocal 9', 'y', 'getlocal 2'),
    SP('getlocal 9', 'width', 'getlocal 3'), SP('getlocal 9', 'height', 'getlocal 4'),
    GP('getlocal 8', 'defaultTextFormat'), 'setlocal 10',
    SP('getlocal 10', 'size', 'getlocal 5'), SP('getlocal 10', 'align', 'getlocal 7'),
    SP('getlocal 9', 'defaultTextFormat', 'getlocal 10'),
    SP('getlocal 9', 'textColor', 'getlocal 6'),
    'getlocal 9', 'returnvalue'))

def card_text(x, y, w, h, size, color, src):
    return CV('getlocal3', 'addChild', MK(x, y, w, h, size, color, 'left', src))

def header(x, y, w, h, size, color, align, text, store=None, src=HEAD):
    out = [MK(x, y, w, h, size, color, align, src), 'setlocal 4', SP('getlocal 4', 'text', f'pushstring "{text}"')]
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
    f'findpropstrict {q("WorkshopItemRendererMovie")}', f'constructprop {q("WorkshopItemRendererMovie")}, 0', 'setlocal 9',
    'newarray 0', 'setlocal 5',
    SP(T, 'hbxCards', 'getlocal 5'),
    T, 'newobject 0', f'setproperty {q("hbxAsked")}',
    N(0), 'setlocal 2', 'jump LCOND',
    'LBODY:', 'label',
    f'findpropstrict {SPRITE}', f'constructprop {SPRITE}, 0', 'setlocal 3',
    SP('getlocal 3', 'x', J(N(24), 'getlocal 2', N(4), 'modulo', N(CARD_W + 16), 'multiply', 'add')),
    SP('getlocal 3', 'y', J(N(104), 'getlocal 2', N(4), 'divide', 'convert_i', N(CARD_H + 14), 'multiply', 'add')),
    SP('getlocal 3', 'buttonMode', 'pushtrue'),
    SP('getlocal 3', 'mouseChildren', 'pushfalse'),
    CV('getlocal 3', 'addEventListener', 'pushstring "click"', GP(T, 'hbxCardClick')),
    card_text(8, PV_H + 12, PV_W, 46, 19, WHITE, ROW_TITLE),
    card_text(8, PV_H + 58, PV_W, 22, 14, WHITE, ROW_DESC),
    card_text(8, PV_H + 78, PV_W, 22, 14, WHITE, ROW_AUTHOR),
    f'findpropstrict {IL}', f'constructprop {IL}, 0', 'setlocal 7',
    f'findpropstrict {SPRITE}', f'constructprop {SPRITE}, 0', 'setlocal 8',
    GP('getlocal 8', 'graphics'), 'setlocal 6',
    CV('getlocal 6', 'beginFill', N(0), N(0)),
    CV('getlocal 6', 'drawRect', N(0), N(0), N(PV_W), N(PV_H)),
    CV('getlocal 6', 'endFill'),
    SP('getlocal 7', 'imageContainer', 'getlocal 8'),
    CV('getlocal 7', 'addChild', 'getlocal 8'),
    SP('getlocal 7', 'x', N(8)), SP('getlocal 7', 'y', N(8)),
    CV('getlocal 3', 'addChild', 'getlocal 7'),
    CV('getlocal 5', 'push', 'getlocal 3').replace(q('push'), BI('push')),
    CV('getlocal1', 'addChild', 'getlocal 3'),
    'inclocal_i 2',
    'LCOND:', 'getlocal 2', N(PER), 'iflt LBODY',
    header(24, 8, 640, 56, 44, WHITE, 'left', 'WORKSHOP DOWNLOADER'),
    header(28, 64, 640, 28, 17, BLUE, 'left', 'Download a workshop map directly into your game!', src=ROW_DESC),
    header(700, 66, 376, 26, 17, BLUE, 'right', '', 'hbxPageLbl', src=ROW_DESC),
    header(24, 606, 1052, 34, 20, WHITE, 'center', '', 'hbxFoot', src=ROW_TITLE),
    'LEND:'))

griddraw = ops(J(
    GP(ROWS, 'length'), 'convert_i', 'setlocal 6',
    N(0), 'setlocal1', 'jump LCOND',
    'LBODY:', 'label',
    GP(T, 'hbxCards'), 'getlocal1', f'getproperty {DYN}', 'setlocal2',
    GP(T, 'hbxPage'), N(PER), 'multiply', 'getlocal1', 'add', 'convert_i', 'setlocal 4',
    'getlocal 4', 'getlocal 6', 'iflt LSHOW',
    SP('getlocal2', 'visible', 'pushfalse'), 'jump LNEXT',
    'LSHOW:',
    SP('getlocal2', 'visible', 'pushtrue'),
    ROWS, 'getlocal 4', f'getproperty {DYN}', 'setlocal3',
    CP('getlocal2', 'getChildAt', N(0)), GP('getlocal3', 'Title'), 'convert_s', f'callproperty {BI("toUpperCase")}, 0', f'setproperty {q("text")}',
    CP('getlocal2', 'getChildAt', N(2)), 'pushstring "Submitted by "', GP('getlocal3', 'AuthorName'), 'convert_s', 'add', f'setproperty {q("text")}',
    GP(T, 'hbxAsked'), GP('getlocal3', 'FullPath'), f'getproperty {DYN}', 'iffalse LNOTASK',
    'pushstring "DOWNLOADING IN HEBNIX..."', 'setlocal 12', 'jump LSET',
    'LNOTASK:', GP('getlocal3', 'Description'), 'convert_s', 'setlocal 12',
    'LSET:',
    CP('getlocal2', 'getChildAt', N(1)), 'getlocal 12', f'setproperty {q("text")}',
    GP('getlocal3', 'Preview'), 'iffalse LNOPV',
    GP('getlocal3', 'Preview'), 'convert_s', 'setlocal 13', 'jump LPV',
    'LNOPV:', 'pushstring ""', 'setlocal 13',
    'LPV:',
    CP('getlocal2', 'getChildAt', N(3)), 'getlocal 13', f'setproperty {q("image")}',
    'getlocal1', GP(T, 'hbxSel'), 'ifne LNOSEL',
    N(3), 'setlocal 8', 'pushint 16777215', 'setlocal 9', 'pushint 13168895', 'setlocal 10',
    'pushint 24462', 'setlocal 14', 'pushint 9868950', 'setlocal 15', 'jump LDRAW',
    'LNOSEL:', N(2), 'setlocal 8', 'pushint 3968200', 'setlocal 9', 'pushint 1854066', 'setlocal 10',
    'pushint 16777215', 'setlocal 14', 'pushint 11056320', 'setlocal 15',
    'LDRAW:',
    SP(CP('getlocal2', 'getChildAt', N(0)), 'textColor', 'getlocal 14'),
    SP(CP('getlocal2', 'getChildAt', N(1)), 'textColor', 'getlocal 15'),
    SP(CP('getlocal2', 'getChildAt', N(2)), 'textColor', 'pushint 4172277'),
    GP('getlocal2', 'graphics'), 'setlocal 5',
    CV('getlocal 5', 'clear'),
    CV('getlocal 5', 'beginFill', 'getlocal 10', 'pushdouble 0.92'),
    CV('getlocal 5', 'lineStyle', 'getlocal 8', 'getlocal 9'),
    CV('getlocal 5', 'drawRect', N(0), N(0), N(CARD_W), N(CARD_H)),
    CV('getlocal 5', 'endFill'),
    CV('getlocal 5', 'lineStyle'),
    CV('getlocal 5', 'beginFill', 'pushint 664120', N(1)),
    CV('getlocal 5', 'drawRect', N(8), N(8), N(PV_W), N(PV_H)),
    CV('getlocal 5', 'endFill'),
    'LNEXT:', 'inclocal_i 1',
    'LCOND:', 'getlocal1', N(PER), 'iflt LBODY',
    'getlocal 6', N(PER - 1), 'add', N(PER), 'divide', 'convert_i', 'setlocal 11',
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
    'getlocal 3', N(PER - 1), 'add', N(PER), 'divide', 'convert_i', 'setlocal 4',
    'getlocal 4', N(1), 'ifge L0', N(1), 'setlocal 4', 'L0:',
    'getlocal 1', N(0), 'ifge L1', N(0), 'setlocal 1', 'L1:',
    'getlocal 1', 'getlocal 4', N(1), 'subtract', 'ifle L2', 'getlocal 4', N(1), 'subtract', 'setlocal 1', 'L2:',
    'getlocal 3', N(1), 'subtract', 'getlocal 1', N(PER), 'multiply', 'subtract', 'setlocal 5',
    'getlocal 5', N(PER - 1), 'ifle L3', N(PER - 1), 'setlocal 5', 'L3:',
    'getlocal 5', N(0), 'ifge L4', N(0), 'setlocal 5', 'L4:',
    'getlocal 2', 'getlocal 5', 'ifle L5', 'getlocal 5', 'setlocal 2', 'L5:',
    'getlocal 2', N(0), 'ifge L6', N(0), 'setlocal 2', 'L6:',
    SP(T, 'hbxPage', J('getlocal 1', 'convert_i')),
    SP(T, 'hbxSel', J('getlocal 2', 'convert_i')),
    CV(T, 'hbxGridDraw')))

gridgo = ops(J(
    GP(T, 'hbxPage'), N(PER), 'multiply', GP(T, 'hbxSel'), 'add', 'convert_i', 'setlocal1',
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
    GP(T, 'hbxSel'), N(PER - 5), 'ifgt LEND',
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
    GP(T, 'hbxBgSave'), 'iftrue LSAVED',
    T, GP(GP(T, 'background'), 'x'), GP(GP(T, 'background'), 'y'),
    GP(GP(T, 'background'), 'width'), GP(GP(T, 'background'), 'height'),
    'newarray 4', f'setproperty {q("hbxBgSave")}',
    'LSAVED:',
    SP(GP(T, 'background'), 'width', J(N(1100), 'getlocal1', 'multiply')),
    SP(GP(T, 'background'), 'height', J(N(650), 'getlocal1', 'multiply')),
    SP(GP(T, 'background'), 'x', J(N(-550), 'getlocal1', 'multiply')),
    SP(GP(T, 'background'), 'y', GP('getlocal2', 'y')),
    CV(T, 'addChild', 'getlocal2'),
    SP('getlocal2', 'visible', 'pushtrue'),
    f'getlex {fq("projectx.api","ShellAPI")}', f'getproperty {q("inputType")}',
    f'getlex {fq("projectx.api","InputTypes")}', f'getproperty {q("PC")}', 'equals', 'iffalse LPADF',
    SP(GP(T, 'hbxFoot'), 'text', 'pushstring "ENTER  DOWNLOAD          PAGE UP / DOWN  PAGE          ESC  BACK"'), 'jump LFOOTD',
    'LPADF:',
    SP(GP(T, 'hbxFoot'), 'text', 'pushstring "GP_A  DOWNLOAD          GP_LShoulder GP_RShoulder  PAGE          GP_Y  SEARCH          GP_B  BACK"'),
    f'getlex {fq("projectx.managers","SystemIconManager")}', GP(T, 'hbxFoot'), 'pushdouble 1.6', N(-2),
    f'callpropvoid {q("replaceMarkupWithIcons")}, 3',
    'LFOOTD:',
    GP(T, 'hbxSearch'), 'iffalse LNS',
    SP(GP(T, 'hbxSearch'), 'x', J(GP('getlocal2', 'x'), N(880), 'getlocal1', 'multiply', 'add')),
    SP(GP(T, 'hbxSearch'), 'y', J(GP('getlocal2', 'y'), N(30), 'getlocal1', 'multiply', 'add')),
    CV(T, 'addChild', GP(T, 'hbxSearch')),
    'LNS:',
    CV(T, 'hbxGridRows'),
    'LEND:'))

def BG(i):
    return J(GP(T, 'hbxBgSave'), N(i), f'getproperty {DYN}')

gridclose = ops(J(
    SP(T, 'hbxDl', 'pushfalse'),
    GP(T, 'hbxBgSave'), 'iffalse LNOBG',
    SP(GP(T, 'background'), 'x', BG(0)), SP(GP(T, 'background'), 'y', BG(1)),
    SP(GP(T, 'background'), 'width', BG(2)), SP(GP(T, 'background'), 'height', BG(3)),
    'LNOBG:',
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

mmethods = ("""