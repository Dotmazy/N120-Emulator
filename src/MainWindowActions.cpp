#include "MainWindow.h"
#include "LogViewer.h"
#include <QtWidgets>
#include <QJsonDocument>
#include <QTcpSocket>
void MainWindow::buildMenus(){
    toolbar=addToolBar("Emulation");toolbar->setObjectName("emulationToolbar");toolbar->setMovable(false);toolbar->setIconSize({20,20});toolbar->setToolButtonStyle(Qt::ToolButtonIconOnly);
    auto file=menuBar()->addMenu("&File"),emulation=menuBar()->addMenu("&Emulation"),hw=menuBar()->addMenu("&Hardware"),view=menuBar()->addMenu("&View"),tools=menuBar()->addMenu("&Tools"),help=menuBar()->addMenu("&Help");
    auto add=[&](QMenu* menu,QString id,QString label,QStyle::StandardPixmap icon,std::function<void()> handler,QKeySequence shortcut={},bool tool=false){auto a=menu->addAction(style()->standardIcon(icon),label);a->setObjectName(id);a->setToolTip(label);a->setShortcut(shortcut);connect(a,&QAction::triggered,this,handler);actions[id]=a;if(tool)toolbar->addAction(a);return a;};
    add(file,"openInternal","Open Internal Firmware…",QStyle::SP_DialogOpenButton,[this]{auto p=QFileDialog::getOpenFileName(this,"Internal firmware",config.internal,"Firmware (*.bin)");if(!p.isEmpty()){config.internal=p;QSettings().setValue("internal",p);recentFiles();configure();}},QKeySequence::Open);
    add(file,"loadExternal","Load External Flash Image…",QStyle::SP_DriveHDIcon,[this]{configure();});
    add(file,"saveFlash","Save External Flash…",QStyle::SP_DialogSaveButton,[this]{saveFlash();});recentMenu=file->addMenu("Recent Firmware Files");file->addSeparator();
    add(file,"screenshot","Screenshot LCD…",QStyle::SP_DesktopIcon,[this]{screenshot(false);},QKeySequence("Ctrl+Shift+S"));
    add(file,"screenshotWindow","Screenshot Full Window…",QStyle::SP_ComputerIcon,[this]{screenshot(true);});file->addSeparator();add(file,"exit","Exit",QStyle::SP_DialogCloseButton,[this]{close();},QKeySequence::Quit);
    add(emulation,"start","Start Simulation",QStyle::SP_MediaPlay,[this]{if(engine.connected())engine.command("resume");else if(!engine.running())power();},{},true);
    add(emulation,"reset","Reset MCU",QStyle::SP_BrowserReload,[this]{releaseKeys();engine.command("reset");},QKeySequence("Ctrl+R"),true);
    add(emulation,"pause","Pause",QStyle::SP_MediaPause,[this]{engine.command(hardware["paused"].toBool()?"resume":"pause");},QKeySequence("F6"),true);
    add(emulation,"resume","Resume",QStyle::SP_MediaPlay,[this]{engine.command("resume");},QKeySequence("F7"));
    add(emulation,"stop","Stop",QStyle::SP_MediaStop,[this]{pendingRestart=false;releaseKeys();engine.stop();},QKeySequence("F8"),true);
    add(emulation,"restart","Reload Firmware",QStyle::SP_BrowserReload,[this]{if(!engine.connected()){if(!engine.running())startWith(config);return;}releaseKeys();auto token=[](const QString& p){return p.isEmpty()?QByteArray("-"):p.toUtf8().toBase64();};statusBar()->showMessage("Reloading firmware…");engine.command("reload "+token(config.internal)+' '+token(config.external)+' '+QByteArray::number(config.externalOffset)+' '+token(config.persistent));},QKeySequence("Ctrl+F5"),true);
    add(emulation,"restartEngine","Restart Emulation Engine",QStyle::SP_BrowserStop,[this]{restart();});
    add(emulation,"speed","Speed Settings…",QStyle::SP_MediaSeekForward,[this]{speedSettings();});
    add(hw,"lcdModel","LCD Model…",QStyle::SP_ComputerIcon,[this]{configure();});
    add(hw,"battery","Battery & Power",QStyle::SP_DriveNetIcon,[this]{sidebar->show();tabs->setCurrentIndex(3);});
    add(hw,"usb","USB Connection",QStyle::SP_DriveNetIcon,[this]{setLayoutMode("Developer");tabs->setCurrentIndex(4);});
    add(hw,"keymap","Keyboard Mapping…",QStyle::SP_FileDialogDetailedView,[this]{settings(3);});
    add(hw,"flash","External Flash Manager",QStyle::SP_DriveHDIcon,[this]{setLayoutMode("Developer");tabs->setCurrentIndex(5);});
    add(view,"standard","Standard Mode",QStyle::SP_ComputerIcon,[this]{setLayoutMode("Standard");},QKeySequence("Ctrl+3"));
    add(view,"compact","Compact Mode",QStyle::SP_TitleBarShadeButton,[this]{setCompact(true);},QKeySequence("Ctrl+1"));
    add(view,"developer","Developer Mode",QStyle::SP_TitleBarUnshadeButton,[this]{setCompact(false);},QKeySequence("Ctrl+2"));
    view->addAction(sidebar->toggleViewAction());view->addAction(toolbar->toggleViewAction());auto status=view->addAction("Show Status Bar");status->setCheckable(true);status->setChecked(true);connect(status,&QAction::toggled,statusBar(),&QStatusBar::setVisible);
    add(view,"zoomIn","Zoom In",QStyle::SP_ArrowUp,[this]{zoom=qMin(2.0,zoom+.1);applyZoom();},QKeySequence::ZoomIn);
    add(view,"zoomOut","Zoom Out",QStyle::SP_ArrowDown,[this]{zoom=qMax(.6,zoom-.1);applyZoom();},QKeySequence::ZoomOut);
    add(view,"zoomReset","Reset Zoom",QStyle::SP_BrowserReload,[this]{zoom=1;applyZoom();});
    add(view,"fullscreen","Fullscreen",QStyle::SP_TitleBarMaxButton,[this]{isFullScreen()?showNormal():showFullScreen();},QKeySequence("F11"));
    add(view,"theme","Theme Selection…",QStyle::SP_DesktopIcon,[this]{settings(1);});
    add(tools,"diagnostics","Diagnostics / Log Viewer",QStyle::SP_FileDialogInfoView,[this]{setLayoutMode("Developer");tabs->setCurrentIndex(6);});
    add(tools,"console","Renode Console…",QStyle::SP_ComputerIcon,[this]{console();});
    add(tools,"export","Export Diagnostic Report…",QStyle::SP_DialogSaveButton,[this]{exportReport();});
    add(tools,"settings","Settings…",QStyle::SP_FileDialogDetailedView,[this]{settings();},QKeySequence("Ctrl+,"),true);
    add(help,"shortcuts","Keyboard Shortcuts",QStyle::SP_DialogHelpButton,[this]{QMessageBox::information(this,"Keyboard shortcuts","Arrows: navigation · F1: OK · F2: Power\nF3: physical Shift · F4: physical Alpha\nEnter: EXE · Esc: Back · Home: Home · Backspace/Delete: erase\nType letters and symbols for physical Alpha/Shift sequences. OS text input supports AZERTY and QWERTY.\nF6: Pause · F7: Resume · F8: Stop\nCtrl+1/2/3: Compact/Developer/Standard · F11: Fullscreen\nCustomize raw key bindings in Settings → Keyboard.");});
    add(help,"guide","User Guide",QStyle::SP_DialogHelpButton,[this]{QDesktopServices::openUrl(QUrl::fromLocalFile(QStringLiteral(N120_SOURCE_DIR)+"/README.md"));});
    add(help,"about","About / Version Information",QStyle::SP_MessageBoxInformation,[this]{QMessageBox::about(this,"N120 Studio","N120 Studio 0.3.0\nC++20 / Qt "+QString(qVersion())+" / Renode\nReal STM32H725 firmware / 320 × 240 LCD\nCalculator artwork provided by the user.\nSee the upgrade report for hardware coverage and USB limitations.");});
    toolbar->insertAction(actions["settings"],actions["screenshot"]);
}
void MainWindow::updateActions(){bool on=engine.connected(),paused=hardware["paused"].toBool();for(auto id:{"reset","pause","resume","stop","saveFlash","console"})actions[id]->setEnabled(on);actions["stop"]->setEnabled(engine.running());actions["pause"]->setEnabled(on);actions["pause"]->setIcon(studioIcon(paused?"start":"pause",qApp->palette().color(QPalette::ButtonText)));actions["pause"]->setText(paused?"Resume":"Pause");actions["pause"]->setToolTip(paused?"Resume simulation":"Pause simulation");actions["resume"]->setEnabled(on&&paused);for(auto id:{"openInternal","loadExternal","lcdModel"})actions[id]->setEnabled(!engine.running());actions["start"]->setEnabled((on&&paused)||(!engine.running()&&!config.internal.isEmpty()));actions["restart"]->setEnabled(!config.internal.isEmpty()&&(on||!engine.running()));actions["restartEngine"]->setEnabled(!config.internal.isEmpty()&&!pendingRestart);if(auto button=findChild<QPushButton*>("usbProbe"))button->setEnabled(on&&!paused&&!hardware["usbPending"].toBool());if(auto button=findChild<QPushButton*>("usbIpStop"))button->setEnabled(on&&hardware["usbIpPort"].toInt()!=0);if(auto button=findChild<QPushButton*>("usbIpStart"))button->setEnabled(on&&!paused&&hardware["usbIpPort"].toInt()==0);}
void MainWindow::power(){if(!engine.running()){if(config.internal.isEmpty())configure();else startWith(config);return;}engine.command("sequence 8");}
void MainWindow::restart(){releaseKeys();if(!engine.running()){startWith(config);return;}pendingRestart=true;engine.stop();}
void MainWindow::setCompact(bool value){setLayoutMode(value?"Compact":"Developer");}
void MainWindow::setLayoutMode(const QString& name){layoutName=name;compact=name=="Compact";bool developer=name=="Developer";sidebar->setWindowTitle(developer?"Hardware inspector":"Session");for(int i=0;i<tabs->count();++i)tabs->setTabVisible(i,developer||i==3);tabs->tabBar()->setVisible(developer);tabs->setCurrentIndex(developer?0:3);sidebar->setVisible(!compact);toolbar->setVisible(!compact);setMinimumWidth(compact?360:790);QSettings().setValue("view/layout",name);if(!isFullScreen())resize(compact?510:1090,height());if(!hardware.isEmpty()){updatePowerCard();if(!developer)liveStatus->setText(QString("● %1  |  %2×  |  %3 s").arg(hardware["paused"].toBool()?"Paused":hardware["cpuWaiting"].toBool()&&!hardware["lcdActive"].toBool()?"Sleeping":"Running").arg(speed,0,'f',2).arg(hardware["virtualMs"].toDouble()/1000,0,'f',1));}}
void MainWindow::applyZoom(){auto area=qobject_cast<QScrollArea*>(centralWidget());area->setAlignment(Qt::AlignCenter);area->setWidgetResizable(qFuzzyCompare(zoom,1.0));surface->setMinimumSize(0,0);if(!qFuzzyCompare(zoom,1.0))surface->resize(area->viewport()->size()*zoom);QSettings().setValue("view/zoom",zoom);}
void MainWindow::recentFiles(){QSettings s;auto files=s.value("general/recent").toStringList();if(!config.internal.isEmpty()){files.removeAll(config.internal);files.prepend(config.internal);}while(files.size()>8)files.removeLast();s.setValue("general/recent",files);recentMenu->clear();for(const auto& path:files){auto a=recentMenu->addAction(QFileInfo(path).fileName());a->setToolTip(path);connect(a,&QAction::triggered,this,[this,path]{if(engine.running()){statusBar()->showMessage("Stop emulation before opening another firmware.",4000);return;}config.internal=path;configure();});}}
void MainWindow::saveFlash(){if(!engine.connected())return;auto p=QFileDialog::getSaveFileName(this,"Save full external flash",{},"Flash image (*.bin)");if(!p.isEmpty())engine.command("save "+p.toUtf8().toBase64());}
void MainWindow::screenshot(bool full){auto p=QFileDialog::getSaveFileName(this,"Save screenshot",full?"n120-window.png":"n120-lcd.png","PNG (*.png)");if(p.isEmpty())return;bool ok=full?grab().save(p):surface->lcdImage().save(p);if(!ok)QMessageBox::warning(this,"Screenshot","Could not write image.");}
void MainWindow::exportReport(){
    auto path=QFileDialog::getSaveFileName(this,"Export diagnostic report","n120-diagnostics.json","JSON (*.json)");
    if(path.isEmpty())return;
    auto state=hardware;state.remove("frame");QString log=logs->text();
    for(auto local:{QDir::homePath(),QStringLiteral(N120_SOURCE_DIR)}){
        log.replace(local,"<local>",Qt::CaseInsensitive);
        log.replace(QDir::toNativeSeparators(local),"<local>",Qt::CaseInsensitive);
    }
    QJsonObject report{{"application","N120 Studio 0.3.0"},{"qt",qVersion()},{"internal",QFileInfo(config.internal).fileName()},{"internalBytes",QFileInfo(config.internal).size()},{"external",QFileInfo(config.external).fileName()},{"externalBytes",QFileInfo(config.external).size()},{"hardware",state},{"logs",log}};
    QSaveFile file(path);if(!file.open(QIODevice::WriteOnly)||file.write(QJsonDocument(report).toJson())<0||!file.commit())QMessageBox::warning(this,"Export failed",file.errorString());
}
void MainWindow::console(){auto d=new QDialog(this);d->setAttribute(Qt::WA_DeleteOnClose);d->setWindowTitle("Renode monitor — live console");d->resize(760,480);auto l=new QVBoxLayout(d);auto output=new QPlainTextEdit;output->setReadOnly(true);output->setMaximumBlockCount(1000);auto input=new QLineEdit;input->setPlaceholderText("Monitor command (e.g. cpu PC)");l->addWidget(output);l->addWidget(input);auto socket=new QTcpSocket(d);connect(socket,&QTcpSocket::readyRead,d,[socket,output]{output->insertPlainText(QString::fromUtf8(socket->readAll()));output->moveCursor(QTextCursor::End);});connect(socket,&QTcpSocket::errorOccurred,d,[socket,output]{output->appendPlainText(socket->errorString());});connect(input,&QLineEdit::returnPressed,d,[socket,input]{socket->write(input->text().toUtf8()+"\r\n");input->clear();});socket->connectToHost(QHostAddress::LocalHost,engine.consolePort());d->show();}
void MainWindow::speedSettings(){bool ok=false;QStringList values={"1× real time","Unlimited","0.25×","0.5×","2×","4×","Custom…"};auto choice=QInputDialog::getItem(this,"Emulation target","Target virtual seconds per wall second",values,0,false,&ok);if(!ok)return;double target=choice==values[0]?1:choice==values[1]?0:choice==values[2]?.25:choice==values[3]?.5:choice==values[4]?2:choice==values[5]?4:QInputDialog::getDouble(this,"Custom speed","Target",1,.05,16,2,&ok);if(ok){QSettings().setValue("emulation/target",target);engine.command("speed "+QByteArray::number(target));}}
