import shutil,os,subprocess
R=os.path.expanduser('~/Documents/hebnix-rl-re/tools/rabc')
def build(name, files):
    shutil.rmtree(name,ignore_errors=True); os.makedirs(name)
    shutil.copytree('rt/v-0', f'{name}/v-0')
    for rel,edits in files.items():
        f=f'{name}/v-0/{rel}'
        s=open(f,encoding='utf-8').read()
        for old,new in edits:
            assert s.count(old)==1,(rel,s.count(old),old[:70])
            s=s.replace(old,new)
        open(f,'w',encoding='utf-8').write(s)
    shutil.copy('in.swf',f'{name}/o.swf')
    subprocess.run([R+'/rabcasm.exe',f'{name}/v-0/v-0.main.asasm'],check=True)
    subprocess.run([R+'/abcreplace.exe',f'{name}/o.swf','0',f'{name}/v-0/v-0.main.abc'],check=True)
