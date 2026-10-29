# RPM do EcoReset (Fedora). Gerado pelo GitHub Actions; para gerar à mão:
#   rpmbuild -bb packaging/ecoreset.spec --define "_sourcedir $PWD" --define "version 0.1.0"
%{!?version: %global version 0.1.0}
%{!?release: %global release 1}
# o código Python fica em /usr/share/ecoreset; não precisa de .pyc pré-compilado
%global __brp_python_bytecompile %{nil}

Name:           ecoreset
Version:        %{version}
Release:        %{release}%{?dist}
Summary:        Manutenção de impressoras Epson L/EcoTank e Canon
License:        Proprietary
URL:            https://github.com/ilmar-coder/mywic
BuildArch:      noarch

Requires:       python3 >= 3.9
Requires:       python3-tkinter
Requires:       libusb1
Requires:       cups-client
Requires:       systemd-udev

%description
EcoReset lê e zera os contadores da almofada de tinta, faz teste de bicos e
limpeza, lê e grava a EEPROM das Epson L/EcoTank (pelo cabo USB ou pela rede)
e diagnostica erros das Canon. Inclui a regra udev que libera o USB das
impressoras Epson e Canon para o usuário logado.

%prep

%build

%install
install -d %{buildroot}%{_datadir}/ecoreset/ecoreset
install -m 0644 %{_sourcedir}/ecoreset/*.py %{buildroot}%{_datadir}/ecoreset/ecoreset/
install -Dm 0755 %{_sourcedir}/packaging/linux/ecoreset.sh %{buildroot}%{_bindir}/ecoreset
install -Dm 0644 %{_sourcedir}/packaging/linux/ecoreset.desktop \
    %{buildroot}%{_datadir}/applications/ecoreset.desktop
install -Dm 0644 %{_sourcedir}/packaging/linux/ecoreset.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/ecoreset.svg
install -Dm 0644 %{_sourcedir}/packaging/linux/70-ecoreset.rules \
    %{buildroot}%{_udevrulesdir}/70-ecoreset.rules

%post
udevadm control --reload-rules >/dev/null 2>&1 || :
udevadm trigger --subsystem-match=usb --attr-match=idVendor=04b8 >/dev/null 2>&1 || :
udevadm trigger --subsystem-match=usb --attr-match=idVendor=04a9 >/dev/null 2>&1 || :

%postun
udevadm control --reload-rules >/dev/null 2>&1 || :

%files
%{_bindir}/ecoreset
%{_datadir}/ecoreset
%{_datadir}/applications/ecoreset.desktop
%{_datadir}/icons/hicolor/scalable/apps/ecoreset.svg
%{_udevrulesdir}/70-ecoreset.rules
