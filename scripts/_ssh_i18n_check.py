import re

key_re = re.compile(r'^\s*"([^"]+)"\s*:\s*"((?:[^"\\]|\\.)*)"', re.M)


def extract_keys(path):
    keys = {}
    txt = open(path, encoding="utf-8").read()
    for m in key_re.finditer(txt):
        keys[m.group(1)] = m.group(2)
    return keys


en = extract_keys("apps/desktop/src/lib/i18n/messages-en.ts")
ru = extract_keys("apps/desktop/src/lib/i18n/messages-ru.ts")


def ssh_related(k):
    return k.startswith("settings.ssh.") or (k.startswith("aiTools.") and "ssh" in k.lower())


en_ssh = {k: v for k, v in en.items() if ssh_related(k)}
ru_ssh = {k: v for k, v in ru.items() if ssh_related(k)}

print("EN ssh keys:", len(en_ssh))
print("RU ssh keys:", len(ru_ssh))
print("ONLY in EN:", sorted(set(en_ssh) - set(ru_ssh)))
print("ONLY in RU:", sorted(set(ru_ssh) - set(en_ssh)))

ph = re.compile(r'\{(\w+)\}')
for k in sorted(set(en_ssh) & set(ru_ssh)):
    pe = set(ph.findall(en_ssh[k]))
    pr = set(ph.findall(ru_ssh[k]))
    if pe != pr:
        print("PLACEHOLDER MISMATCH", k, "EN=", sorted(pe), "RU=", sorted(pr))
print("placeholder check done")

# keys used by SshSection.tsx
comp = open("apps/desktop/src/components/settings/SshSection.tsx", encoding="utf-8").read()
used = set(re.findall(r't\(\s*"([^"]+)"', comp))
used_ssh = {k for k in used if k.startswith("settings.ssh.")}
print("SshSection.tsx settings.ssh.* keys used:", sorted(used_ssh))
missing_en = used_ssh - set(en)
missing_ru = used_ssh - set(ru)
print("USED-but-missing in EN:", sorted(missing_en))
print("USED-but-missing in RU:", sorted(missing_ru))
