//+------------------------------------------------------------------+
//| TradeLensSync.mq5                                                |
//| Gửi deal của tài khoản sang app TradeLens qua HTTP 127.0.0.1.    |
//| Hợp đồng API: docs/sync-api.md                                   |
//+------------------------------------------------------------------+
#property copyright "TradeLens"
#property link      "https://github.com/DEVfancybear/tradelens"
#property version   "0.10"
#property strict

#include <TradeLens/Json.mqh>
#include <TradeLens/Http.mqh>

//--- Cài đặt (Tools → Options → Expert Advisors: Allow WebRequest cho http://127.0.0.1:7878)
input string InpToken        = "";    // Token (copy từ app: Nguồn dữ liệu)
input int    InpPort         = 7878;  // Cổng bridge
input int    InpBackfillDays = 90;    // Số ngày lịch sử gửi lần đầu
input int    InpTimerSeconds = 60;    // Chu kỳ gửi bù (giây)
input bool   InpScreenshots  = true;  // Tự chụp chart lúc vào và ra lệnh
input ENUM_TIMEFRAMES InpScreenshotTf = PERIOD_M15; // Khung giờ ưu tiên khi chụp
input int    InpScreenshotW  = 1600;  // Rộng ảnh (px)
input int    InpScreenshotH  = 900;   // Cao ảnh (px)

//--- Deal mới chờ gửi (ticket), gom từ OnTradeTransaction
ulong g_pending[];
datetime g_last_sync = 0;

//+------------------------------------------------------------------+
int OnInit()
  {
   if(StringLen(InpToken) == 0)
     {
      Print("TradeLens: chưa nhập token");
      return(INIT_PARAMETERS_INCORRECT);
     }
   EventSetTimer(MathMax(InpTimerSeconds, 10));
   // TODO(v0.1): gửi lịch sử InpBackfillDays ngày ngay khi gắn EA.
   return(INIT_SUCCEEDED);
  }

//+------------------------------------------------------------------+
void OnDeinit(const int reason)
  {
   EventKillTimer();
  }

//+------------------------------------------------------------------+
//| Mỗi deal mới (vào, ra, nạp/rút) được xếp hàng để gửi.            |
//+------------------------------------------------------------------+
void OnTradeTransaction(const MqlTradeTransaction &trans,
                        const MqlTradeRequest &request,
                        const MqlTradeResult &result)
  {
   if(trans.type != TRADE_TRANSACTION_DEAL_ADD)
      return;
   int n = ArraySize(g_pending);
   ArrayResize(g_pending, n + 1);
   g_pending[n] = trans.deal;
   if(InpScreenshots && HistoryDealSelect(trans.deal))
     {
      ENUM_DEAL_ENTRY entry = (ENUM_DEAL_ENTRY)HistoryDealGetInteger(trans.deal, DEAL_ENTRY);
      long position = HistoryDealGetInteger(trans.deal, DEAL_POSITION_ID);
      string symbol = HistoryDealGetString(trans.deal, DEAL_SYMBOL);
      if(entry == DEAL_ENTRY_IN)
         SendScreenshot(symbol, position, "entry");
      else if(!PositionSelectByTicket(position))
         SendScreenshot(symbol, position, "exit");   // đóng hẳn, không phải chốt một phần
     }
  }

//+------------------------------------------------------------------+
//| Chụp chart của symbol và gửi sang app.                           |
//+------------------------------------------------------------------+
void SendScreenshot(const string symbol, const long position, const string moment)
  {
   long chart = FindChart(symbol);
   bool temporary = (chart == 0);
   if(temporary)
      chart = ChartOpen(symbol, InpScreenshotTf);
   if(chart == 0)
      return;
   ChartRedraw(chart);
   string file = StringFormat("TradeLens\\%I64d-%s.png", position, moment);
   if(ChartScreenShot(chart, file, InpScreenshotW, InpScreenshotH, ALIGN_RIGHT))
     {
      string headers = StringFormat("X-TradeLens-Login: %I64d\r\nX-TradeLens-Server: %s\r\nX-TradeLens-Position: %I64d\r\nX-TradeLens-Moment: %s\r\n",
                                    AccountInfoInteger(ACCOUNT_LOGIN), AccountInfoString(ACCOUNT_SERVER), position, moment);
      TradeLensPostFile(InpPort, InpToken, "/api/v1/screenshot", headers, file);
      FileDelete(file);
     }
   if(temporary)
      ChartClose(chart);
  }

//+------------------------------------------------------------------+
//| Chart đang mở symbol, ưu tiên khung giờ InpScreenshotTf; 0 nếu không có. |
//+------------------------------------------------------------------+
long FindChart(const string symbol)
  {
   long found = 0;
   for(long chart = ChartFirst(); chart >= 0; chart = ChartNext(chart))
     {
      if(ChartSymbol(chart) != symbol)
         continue;
      if(ChartPeriod(chart) == InpScreenshotTf)
         return(chart);
      if(found == 0)
         found = chart;
     }
   return(found);
  }

//+------------------------------------------------------------------+
//| Gửi hàng đợi; nếu trống thì gửi bù lịch sử từ lần sync cuối.     |
//+------------------------------------------------------------------+
void OnTimer()
  {
   // TODO(v0.1):
   //  1. HistorySelect(g_last_sync - 3600, TimeCurrent())
   //  2. TradeLensDealJson() cho từng deal (kèm risk tính từ SL, tick value)
   //  3. TradeLensPost("/api/v1/deals", TradeLensBatchJson(deals)); thành công thì xoá hàng đợi
  }

//+------------------------------------------------------------------+
//| Độ lệch giờ server so với UTC, phút (gửi kèm account).           |
//+------------------------------------------------------------------+
int ServerOffsetMinutes()
  {
   return((int)((TimeTradeServer() - TimeGMT()) / 60));
  }

//+------------------------------------------------------------------+
//| Tiền rủi ro tại SL của một deal vào lệnh.                        |
//+------------------------------------------------------------------+
double RiskMoney(const string symbol, const double price, const double sl, const double volume)
  {
   if(sl <= 0.0)
      return(0.0);
   double tick_size  = SymbolInfoDouble(symbol, SYMBOL_TRADE_TICK_SIZE);
   double tick_value = SymbolInfoDouble(symbol, SYMBOL_TRADE_TICK_VALUE_LOSS);
   if(tick_size <= 0.0)
      return(0.0);
   return(MathAbs(price - sl) / tick_size * tick_value * volume);
  }
//+------------------------------------------------------------------+
