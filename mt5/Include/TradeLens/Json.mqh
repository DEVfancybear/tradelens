//+------------------------------------------------------------------+
//| Json.mqh — dựng JSON DealBatch (docs/sync-api.md) không cần DLL. |
//+------------------------------------------------------------------+
#property strict

//--- Thoát chuỗi theo JSON (", \, xuống dòng, tab và mọi ký tự điều khiển < 0x20).
//--- Unicode giữ nguyên: body được đổi sang UTF-8 khi gửi (xem Http.mqh).
string TradeLensEscape(const string s)
  {
   string out = "";
   int n = StringLen(s);
   for(int i = 0; i < n; i++)
     {
      ushort c = StringGetCharacter(s, i);
      switch(c)
        {
         case '\\': out += "\\\\"; break;
         case '\"': out += "\\\""; break;
         case '\n': out += "\\n";  break;
         case '\r': out += "\\r";  break;
         case '\t': out += "\\t";  break;
         default:
            if(c < 32)
               out += StringFormat("\\u%04x", (int)c);
            else
               out += ShortToString(c);
        }
     }
   return(out);
  }

//--- Số thực cho JSON: luôn dấu chấm, không bao giờ "nan"/"inf".
string TradeLensNum(const double value, const int digits)
  {
   if(!MathIsValidNumber(value))
      return("0");
   return(DoubleToString(value, digits));
  }

//--- Tên kiểu deal theo ENUM_DEAL_TYPE (so theo số để không phụ thuộc hằng số của bản build).
string TradeLensDealType(const long type)
  {
   switch((int)type)
     {
      case 0:  return("buy");
      case 1:  return("sell");
      case 2:  return("balance");
      case 3:  return("credit");
      case 4:  return("charge");
      case 5:  return("correction");
      case 6:  return("bonus");
      case 7:                        // COMMISSION
      case 8:                        // COMMISSION_DAILY
      case 9:                        // COMMISSION_MONTHLY
      case 10:                       // COMMISSION_AGENT_DAILY
      case 11: return("commission"); // COMMISSION_AGENT_MONTHLY
     }
   return("other");                  // lãi, cổ tức, thuế, huỷ...: nhật ký không dùng
  }

//--- Tên hướng deal theo ENUM_DEAL_ENTRY.
string TradeLensDealEntry(const long entry)
  {
   switch((int)entry)
     {
      case 0:  return("in");
      case 1:  return("out");
      case 2:  return("inout");
      case 3:  return("out_by");
     }
   return("");
  }

//--- Tên lý do deal theo ENUM_DEAL_REASON.
string TradeLensDealReason(const long reason)
  {
   switch((int)reason)
     {
      case 0:  return("client");
      case 1:  return("mobile");
      case 2:  return("web");
      case 3:  return("expert");
      case 4:  return("sl");
      case 5:  return("tp");
      case 6:  return("so");
      case 7:  return("rollover");
      case 8:  return("vmargin");
      case 9:  return("split");
     }
   return("");
  }

//--- Phần "account" của batch (đối tượng AccountInfo).
string TradeLensAccountJson(const int server_offset_min)
  {
   string json = "{";
   json += "\"login\":\"" + IntegerToString(AccountInfoInteger(ACCOUNT_LOGIN)) + "\"";   // app nhận chuỗi
   json += ",\"server\":\"" + TradeLensEscape(AccountInfoString(ACCOUNT_SERVER)) + "\"";
   json += ",\"broker\":\"" + TradeLensEscape(AccountInfoString(ACCOUNT_COMPANY)) + "\"";
   json += ",\"name\":\"" + TradeLensEscape(AccountInfoString(ACCOUNT_NAME)) + "\"";
   json += ",\"currency\":\"" + TradeLensEscape(AccountInfoString(ACCOUNT_CURRENCY)) + "\"";
   json += ",\"balance\":" + TradeLensNum(AccountInfoDouble(ACCOUNT_BALANCE), 8);
   json += ",\"equity\":" + TradeLensNum(AccountInfoDouble(ACCOUNT_EQUITY), 8);
   json += ",\"server_offset_min\":" + IntegerToString(server_offset_min);
   json += "}";
   return(json);
  }

//--- Một deal của lịch sử. sl/tp/risk do EA tính (xem DealLevels trong TradeLensSync.mq5).
//--- Trả "" khi không đọc được deal.
string TradeLensDealJson(const ulong ticket, const double sl, const double tp, const double risk)
  {
   if(!HistoryDealSelect(ticket))
      return("");
   string symbol = HistoryDealGetString(ticket, DEAL_SYMBOL);
   long digits_value = 0;
   int digits = 8;                         // deal tiền (balance...) không có symbol
   if(StringLen(symbol) > 0 && SymbolInfoInteger(symbol, SYMBOL_DIGITS, digits_value))
      digits = (int)digits_value;

   string json = "{";
   json += "\"ticket\":" + StringFormat("%I64u", ticket);
   json += ",\"order\":" + StringFormat("%I64d", HistoryDealGetInteger(ticket, DEAL_ORDER));
   json += ",\"position_id\":" + StringFormat("%I64d", HistoryDealGetInteger(ticket, DEAL_POSITION_ID));
   json += ",\"time_msc\":" + StringFormat("%I64d", HistoryDealGetInteger(ticket, DEAL_TIME_MSC));   // giờ server
   json += ",\"symbol\":\"" + TradeLensEscape(symbol) + "\"";
   json += ",\"type\":\"" + TradeLensDealType(HistoryDealGetInteger(ticket, DEAL_TYPE)) + "\"";
   json += ",\"entry\":\"" + TradeLensDealEntry(HistoryDealGetInteger(ticket, DEAL_ENTRY)) + "\"";
   json += ",\"volume\":" + TradeLensNum(HistoryDealGetDouble(ticket, DEAL_VOLUME), 8);
   json += ",\"price\":" + TradeLensNum(HistoryDealGetDouble(ticket, DEAL_PRICE), digits);
   json += ",\"commission\":" + TradeLensNum(HistoryDealGetDouble(ticket, DEAL_COMMISSION), 8);
   json += ",\"swap\":" + TradeLensNum(HistoryDealGetDouble(ticket, DEAL_SWAP), 8);
   json += ",\"fee\":" + TradeLensNum(HistoryDealGetDouble(ticket, DEAL_FEE), 8);
   json += ",\"profit\":" + TradeLensNum(HistoryDealGetDouble(ticket, DEAL_PROFIT), 8);
   json += ",\"magic\":" + StringFormat("%I64d", HistoryDealGetInteger(ticket, DEAL_MAGIC));
   json += ",\"comment\":\"" + TradeLensEscape(HistoryDealGetString(ticket, DEAL_COMMENT)) + "\"";
   json += ",\"sl\":" + TradeLensNum(sl, digits);
   json += ",\"tp\":" + TradeLensNum(tp, digits);
   json += ",\"risk\":" + TradeLensNum(risk, 2);
   json += ",\"reason\":\"" + TradeLensDealReason(HistoryDealGetInteger(ticket, DEAL_REASON)) + "\"";
   json += "}";
   return(json);
  }

//--- Batch: {"account": {...}, "deals": [deals[first] .. deals[first+count-1]]}.
string TradeLensBatchJson(const string account_json, const string &deals[], const int first, const int count)
  {
   string json = "{\"account\":" + account_json + ",\"deals\":[";
   for(int i = 0; i < count; i++)
     {
      if(i > 0)
         json += ",";
      json += deals[first + i];
     }
   json += "]}";
   return(json);
  }
//+------------------------------------------------------------------+
