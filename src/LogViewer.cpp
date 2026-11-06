#include "LogViewer.h"
#include <QtWidgets>
LogViewer::LogViewer(QWidget* parent):QWidget(parent){
    refreshTimer.setSingleShot(true);refreshTimer.setInterval(50);connect(&refreshTimer,&QTimer::timeout,this,&LogViewer::refresh);
    auto layout=new QVBoxLayout(this);layout->setContentsMargins(0,0,0,0);auto tools=new QHBoxLayout;
    search=new QLineEdit;search->setPlaceholderText("Search logs…");severity=new QComboBox;severity->addItems({"All","Warning","Error","Info"});follow=new QCheckBox("Follow");follow->setChecked(true);
    tools->addWidget(search,1);tools->addWidget(severity);tools->addWidget(follow);layout->addLayout(tools);
    editor=new QPlainTextEdit;editor->setReadOnly(true);editor->setMaximumBlockCount(limit);editor->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));layout->addWidget(editor);
    auto bottom=new QHBoxLayout;auto copy=new QPushButton("Copy"),clear=new QPushButton("Clear"),save=new QPushButton("Export…");bottom->addWidget(copy);bottom->addWidget(clear);bottom->addStretch();bottom->addWidget(save);layout->addLayout(bottom);
    connect(search,&QLineEdit::textChanged,this,[this]{refresh();});connect(severity,&QComboBox::currentIndexChanged,this,[this]{refresh();});
    connect(copy,&QPushButton::clicked,this,[this]{QApplication::clipboard()->setText(editor->toPlainText());});connect(clear,&QPushButton::clicked,this,[this]{records.clear();refresh();});
    connect(save,&QPushButton::clicked,this,[this]{auto p=QFileDialog::getSaveFileName(this,"Export logs",{},"Text (*.log)");if(p.isEmpty())return;QSaveFile f(p);if(!f.open(QIODevice::WriteOnly)||f.write(text().toUtf8())<0||!f.commit())QMessageBox::warning(this,"Export failed",f.errorString());});
}
void LogViewer::append(const QString& text){for(auto line:text.split('\n')){line=line.trimmed();if(line.isEmpty())continue;if(!line.startsWith('['))line="["+QTime::currentTime().toString("HH:mm:ss.zzz")+"] [INFO] "+line;records.append(line);}while(records.size()>limit)records.removeFirst();if(!refreshTimer.isActive())refreshTimer.start();}
QString LogViewer::text() const{return records.join('\n');}
void LogViewer::setLimit(int n){limit=qBound(100,n,10000);editor->setMaximumBlockCount(limit);while(records.size()>limit)records.removeFirst();refresh();}
void LogViewer::refresh(){auto scroll=editor->verticalScrollBar()->value();QStringList visible;auto level=severity->currentText();for(const auto& line:records)if((search->text().isEmpty()||line.contains(search->text(),Qt::CaseInsensitive))&&(severity->currentIndex()==0||line.contains(level,Qt::CaseInsensitive)))visible<<line;editor->setPlainText(visible.join('\n'));editor->verticalScrollBar()->setValue(follow->isChecked()?editor->verticalScrollBar()->maximum():scroll);}
